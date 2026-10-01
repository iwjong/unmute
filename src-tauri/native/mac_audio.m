#import <Foundation/Foundation.h>
#import <AVFoundation/AVFoundation.h>
#import <CoreAudio/CoreAudio.h>
#import <CoreAudio/AudioHardwareTapping.h>
#import <CoreAudio/CATapDescription.h>
#import <mach/mach_time.h>
#import <mach/mach.h>
#include <stdatomic.h>
#include <stdlib.h>
#include <string.h>
#include <math.h>

typedef void (*CaptureCallback)(void *, const float *, uint32_t, uint64_t, double, uint32_t);
typedef struct {
    AudioObjectID device, tap;
    AudioDeviceIOProcID proc;
    AudioStreamBasicDescription format;
    CaptureCallback callback;
    void *context;
    bool aggregate;
    _Atomic uint64_t invalid;
    _Atomic bool stopping;
    float mono[32768];
} Capture;

static OSStatus property(AudioObjectID id, AudioObjectPropertySelector selector,
                         AudioObjectPropertyScope scope, void *value, UInt32 size) {
    AudioObjectPropertyAddress address = {selector, scope, kAudioObjectPropertyElementMain};
    return AudioObjectGetPropertyData(id, &address, 0, NULL, &size, value);
}
static AudioObjectID default_device(bool input) {
    AudioObjectID id = 0;
    property(kAudioObjectSystemObject, input ? kAudioHardwarePropertyDefaultInputDevice :
             kAudioHardwarePropertyDefaultOutputDevice, kAudioObjectPropertyScopeGlobal, &id, sizeof(id));
    return id;
}
static NSString *string_property(AudioObjectID id, AudioObjectPropertySelector selector) {
    CFStringRef value = NULL;
    if (property(id, selector, kAudioObjectPropertyScopeGlobal, &value, sizeof(value)) || !value) return @"";
    return CFBridgingRelease(value);
}
static NSDictionary *route(void) {
    AudioObjectID id = default_device(false);
    NSString *name = string_property(id, kAudioObjectPropertyName);
    NSString *uid = string_property(id, kAudioDevicePropertyDeviceUID);
    UInt32 transport = 0, dataSource = 0, terminal = 0;
    property(id, kAudioDevicePropertyTransportType, kAudioObjectPropertyScopeGlobal, &transport, sizeof(transport));
    property(id, kAudioDevicePropertyDataSource, kAudioDevicePropertyScopeOutput, &dataSource, sizeof(dataSource));
    AudioObjectPropertyAddress address = {kAudioDevicePropertyStreams, kAudioDevicePropertyScopeOutput, kAudioObjectPropertyElementMain};
    UInt32 size = 0;
    if (!AudioObjectGetPropertyDataSize(id, &address, 0, NULL, &size) && size >= sizeof(AudioStreamID)) {
        AudioStreamID *streams = malloc(size);
        if (streams && !AudioObjectGetPropertyData(id, &address, 0, NULL, &size, streams))
            property(streams[0], kAudioStreamPropertyTerminalType, kAudioObjectPropertyScopeGlobal, &terminal, sizeof(terminal));
        free(streams);
    }
    BOOL headphones = terminal == kAudioStreamTerminalTypeHeadphones || dataSource == 'hdpn';
    BOOL speakers = terminal == kAudioStreamTerminalTypeSpeaker || dataSource == 'ispk' ||
        ([name.lowercaseString containsString:@"speaker"] && transport == kAudioDeviceTransportTypeBuiltIn);
    AudioStreamBasicDescription format = {0};
    property(id, kAudioDevicePropertyStreamFormat, kAudioDevicePropertyScopeOutput, &format, sizeof(format));
    return @{ @"id": @(id), @"uid": uid, @"name": name, @"transport": @(transport),
              @"data_source": @(dataSource), @"terminal": @(terminal),
              @"kind": speakers ? @"SPEAKERS" : headphones ? @"HEADPHONES" : @"UNKNOWN",
              @"sample_rate": @(format.mSampleRate), @"channels": @(format.mChannelsPerFrame) };
}
char *um_preflight(void) {
    @autoreleasepool {
        AVAuthorizationStatus status = [AVCaptureDevice authorizationStatusForMediaType:AVMediaTypeAudio];
        NSArray *states = @[@"NOT_DETERMINED", @"RESTRICTED", @"DENIED", @"GRANTED"];
        AudioStreamBasicDescription inputFormat = {0};
        property(default_device(true), kAudioDevicePropertyStreamFormat, kAudioDevicePropertyScopeInput, &inputFormat, sizeof(inputFormat));
        NSDictionary *result = @{ @"microphone": states[status],
            @"system_audio": @"UNKNOWN", @"system_audio_evidence": @"Human confirmation required; no public tap authorization query",
            @"route": route(), @"input_name": string_property(default_device(true), kAudioObjectPropertyName),
            @"input_id": @(default_device(true)), @"input_sample_rate": @(inputFormat.mSampleRate),
            @"input_channels": @(inputFormat.mChannelsPerFrame) };
        NSData *data = [NSJSONSerialization dataWithJSONObject:result options:0 error:nil];
        return strdup([[NSString alloc] initWithData:data encoding:NSUTF8StringEncoding].UTF8String);
    }
}
void um_free(char *s) { free(s); }
void um_timebase(uint32_t *numer, uint32_t *denom, uint64_t *origin) {
    mach_timebase_info_data_t info;
    mach_timebase_info(&info);
    *numer = info.numer; *denom = info.denom; *origin = mach_absolute_time();
}
int um_request_microphone(void) {
    dispatch_semaphore_t done = dispatch_semaphore_create(0);
    __block BOOL granted = NO;
    [AVCaptureDevice requestAccessForMediaType:AVMediaTypeAudio completionHandler:^(BOOL allowed) {
        granted = allowed; dispatch_semaphore_signal(done);
    }];
    if (dispatch_semaphore_wait(done, dispatch_time(DISPATCH_TIME_NOW, 120 * NSEC_PER_SEC))) return -1;
    return granted ? 1 : 0;
}

// Only format decoding/downmixing and a bounded handoff happen on the IO thread.
static float sample(const unsigned char *p, const AudioStreamBasicDescription *f) {
    if (f->mFormatFlags & kAudioFormatFlagIsFloat) {
        if (f->mBitsPerChannel == 32) { float v; memcpy(&v, p, 4); return isfinite(v) ? v : 0; }
        double v; memcpy(&v, p, 8); return isfinite(v) ? (float)v : 0;
    }
    if (f->mBitsPerChannel == 16) { int16_t v; memcpy(&v, p, 2); return v / 32768.f; }
    if (f->mBitsPerChannel == 24) {
        int32_t v = (int32_t)((uint32_t)p[0] << 8 | (uint32_t)p[1] << 16 | (uint32_t)p[2] << 24);
        return v / 2147483648.f;
    }
    int32_t v; memcpy(&v, p, 4); return v / 2147483648.f;
}
static OSStatus capture_io(AudioObjectID device, const AudioTimeStamp *now,
    const AudioBufferList *input, const AudioTimeStamp *inputTime,
    AudioBufferList *output, const AudioTimeStamp *outputTime, void *context) {
    (void)device; (void)now; (void)outputTime;
    Capture *c = context;
    if (atomic_load(&c->stopping)) return noErr;
    // A tap-only aggregate has no output, but never leave an output buffer uninitialized.
    if (output) for (UInt32 b = 0; b < output->mNumberBuffers; ++b)
        if (output->mBuffers[b].mData) memset(output->mBuffers[b].mData, 0, output->mBuffers[b].mDataByteSize);
    if (!input || !input->mNumberBuffers) return noErr;
    if (!inputTime || !(inputTime->mFlags & kAudioTimeStampHostTimeValid)) {
        atomic_fetch_add(&c->invalid, 1); return noErr;
    }
    UInt32 bytes = c->format.mBitsPerChannel / 8, frames = UINT32_MAX, channels = 0;
    for (UInt32 b = 0; b < input->mNumberBuffers; ++b) {
        const AudioBuffer *buffer = &input->mBuffers[b];
        if (!buffer->mNumberChannels || !buffer->mData) { atomic_fetch_add(&c->invalid, 1); return noErr; }
        UInt32 n = buffer->mDataByteSize / (bytes * buffer->mNumberChannels);
        if (frames != UINT32_MAX && frames != n) { atomic_fetch_add(&c->invalid, 1); return noErr; }
        frames = n; channels += buffer->mNumberChannels;
    }
    if (!frames) return noErr;
    if (frames > 32768 || channels != c->format.mChannelsPerFrame) { atomic_fetch_add(&c->invalid, 1); return noErr; }
    memset(c->mono, 0, frames * sizeof(float));
    for (UInt32 b = 0; b < input->mNumberBuffers; ++b) {
        const AudioBuffer *buffer = &input->mBuffers[b];
        const unsigned char *data = buffer->mData;
        for (UInt32 n = 0; n < frames; ++n)
            for (UInt32 ch = 0; ch < buffer->mNumberChannels; ++ch)
                c->mono[n] += sample(data + (n * buffer->mNumberChannels + ch) * bytes, &c->format) / channels;
    }
    c->callback(c->context, c->mono, frames, inputTime->mHostTime, c->format.mSampleRate, channels);
    return noErr;
}
int32_t um_stop(void *handle) {
    Capture *c = handle;
    if (!c) return 0;
    atomic_store(&c->stopping, true);
    if (c->proc) {
        AudioDeviceStop(c->device, c->proc);
        OSStatus status = AudioDeviceDestroyIOProcID(c->device, c->proc);
        // If HAL cannot detach IO, keep the callback and its context alive.
        // Rust records a teardown failure; never free memory a callback might still reference.
        if (status) return status;
    }
    OSStatus aggregateStatus = c->aggregate ? AudioHardwareDestroyAggregateDevice(c->device) : noErr;
    OSStatus tapStatus = c->tap ? AudioHardwareDestroyProcessTap(c->tap) : noErr;
    free(c);
    return aggregateStatus ? aggregateStatus : tapStatus;
}
uint64_t um_invalid(void *handle) { return atomic_load(&((Capture *)handle)->invalid); }
int um_alive(void *handle) {
    UInt32 alive = 0; Capture *c = handle;
    return !property(c->device, kAudioDevicePropertyDeviceIsAlive, kAudioObjectPropertyScopeGlobal, &alive, sizeof(alive)) && alive;
}
void *um_start(int remote, CaptureCallback callback, void *context, int32_t *error) {
    @autoreleasepool {
        Capture *c = calloc(1, sizeof(Capture));
        if (!c) { *error = -108; return NULL; }
        c->callback = callback; c->context = context;
        OSStatus status = noErr;
        if (remote) {
            CATapDescription *description = [[CATapDescription alloc] initStereoGlobalTapButExcludeProcesses:@[]];
            description.name = @"Unmute system output";
            description.privateTap = YES;
            description.muteBehavior = CATapUnmuted;
            status = AudioHardwareCreateProcessTap(description, &c->tap);
            if (status) goto failed;
            status = property(c->tap, kAudioTapPropertyFormat, kAudioObjectPropertyScopeGlobal, &c->format, sizeof(c->format));
            if (status) goto failed;
            // A private tap-only aggregate avoids physical-device drift correction.
            NSDictionary *aggregate = @{
                @kAudioAggregateDeviceNameKey: @"Unmute private tap",
                @kAudioAggregateDeviceUIDKey: NSUUID.UUID.UUIDString,
                @kAudioAggregateDeviceIsPrivateKey: @YES,
                @kAudioAggregateDeviceTapAutoStartKey: @YES,
                @kAudioAggregateDeviceTapListKey: @[@{
                    @kAudioSubTapUIDKey: description.UUID.UUIDString,
                    @kAudioSubTapDriftCompensationKey: @NO }]
            };
            status = AudioHardwareCreateAggregateDevice((__bridge CFDictionaryRef)aggregate, &c->device);
            if (status) goto failed;
            c->aggregate = true;
        } else {
            c->device = default_device(true);
            status = property(c->device, kAudioDevicePropertyStreamFormat, kAudioDevicePropertyScopeInput, &c->format, sizeof(c->format));
            if (status) goto failed;
        }
        if (c->format.mFormatID != kAudioFormatLinearPCM ||
            c->format.mFormatFlags & kAudioFormatFlagIsBigEndian ||
            !(c->format.mFormatFlags & kAudioFormatFlagIsPacked) ||
            !(c->format.mSampleRate >= 8000 && c->format.mSampleRate <= 384000) ||
            !c->format.mChannelsPerFrame || c->format.mChannelsPerFrame > 64 ||
            ((c->format.mFormatFlags & kAudioFormatFlagIsFloat) ?
                !(c->format.mBitsPerChannel == 32 || c->format.mBitsPerChannel == 64) :
                (!(c->format.mFormatFlags & kAudioFormatFlagIsSignedInteger) ||
                !(c->format.mBitsPerChannel == 16 || c->format.mBitsPerChannel == 24 || c->format.mBitsPerChannel == 32)))) {
            status = kAudioDeviceUnsupportedFormatError; goto failed;
        }
        status = AudioDeviceCreateIOProcID(c->device, capture_io, c, &c->proc);
        if (status) goto failed;
        status = AudioDeviceStart(c->device, c->proc);
        if (status) goto failed;
        *error = 0; return c;
    failed:
        *error = status; um_stop(c); return NULL;
    }
}
// Process CPU time is cumulative; Rust derives interval utilization.
int um_resources(uint64_t *memory, double *cpu_seconds) {
    struct mach_task_basic_info basic; mach_msg_type_number_t count = MACH_TASK_BASIC_INFO_COUNT;
    if (task_info(mach_task_self(), MACH_TASK_BASIC_INFO, (task_info_t)&basic, &count)) return -1;
    task_thread_times_info_data_t times; count = TASK_THREAD_TIMES_INFO_COUNT;
    if (task_info(mach_task_self(), TASK_THREAD_TIMES_INFO, (task_info_t)&times, &count)) return -1;
    *memory = basic.resident_size;
    *cpu_seconds = basic.user_time.seconds + basic.user_time.microseconds / 1e6 +
        basic.system_time.seconds + basic.system_time.microseconds / 1e6 +
        times.user_time.seconds + times.user_time.microseconds / 1e6 +
        times.system_time.seconds + times.system_time.microseconds / 1e6;
    return 0;
}
