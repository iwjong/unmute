import assert from 'node:assert/strict';
import { mockAt, recentTurns, nextMoment, moments } from '../src/ui/meeting/mock.ts';

assert.deepEqual(new Set(moments.map(m => m.state.say.state)), new Set(['IDLE', 'THINKING', 'READY', 'STALE', 'ERROR']));
const partial = mockAt(4).turns.at(-1), final = mockAt(8).turns.at(-1);
assert.equal(partial.id, final.id);
assert.equal(partial.status, 'PARTIAL');
assert.equal(final.status, 'FINAL');
assert.equal(mockAt(24).turns.at(-1).id, mockAt(28).turns.at(-1).id);
assert.equal(mockAt(28).turns.at(-1).status, 'FINAL');
assert.equal(mockAt(24).say.text, mockAt(12).say.text);
assert.equal(mockAt(24).say.state, 'STALE');
assert.equal(recentTurns(mockAt(36)).length, 6);
assert.equal(mockAt(36).turns.length, 7); // Older state is retained outside the live view.
assert.equal(recentTurns(mockAt(54)).at(-1).id, '8');
assert.equal(nextMoment(48), 54);
assert.equal(nextMoment(60), 0);
for (const moment of moments) {
  assert.ok(moment.state.turns.every(t => ['REMOTE', 'YOU'].includes(t.speaker)));
  assert.ok(recentTurns(moment.state).length <= 6);
}
console.log('Meeting mock checks passed: all states, partial replacement, recent context, deterministic progression.');

const { applyLocalEvent, emptyMeeting } = await import('../src/ui/meeting/local.ts');
let live = applyLocalEvent(emptyMeeting, { type: 'transcript', id: '0', text: 'How does', final: false });
live = applyLocalEvent(live, { type: 'transcript', id: '0', text: 'How does it work?', final: true });
assert.equal(live.turns.length, 1);
assert.equal(live.turns[0].status, 'FINAL');
live = applyLocalEvent(live, { type: 'say', state: 'READY', text: 'Could you clarify the scope?' });
live = applyLocalEvent(live, { type: 'say', state: 'STALE' });
assert.equal(live.say.state, 'STALE');
assert.equal(live.say.text, 'Could you clarify the scope?');
for (let i = 1; i < 20; i++) live = applyLocalEvent(live, { type: 'transcript', id: String(i), text: 'A turn', final: true });
assert.equal(live.turns.filter(t => t.speaker === 'REMOTE').length, 20);
assert.equal(live.turns[0].text, 'How does it work?');
const history = live.turns;
for (const event of [
  { type: 'say', state: 'THINKING' },
  { type: 'say', state: 'READY', text: 'A response' },
  { type: 'say', state: 'STALE' },
  { type: 'status', status: 'Listening' },
  { type: 'status', status: 'Stopped' },
]) {
  live = applyLocalEvent(live, event);
  assert.deepEqual(live.turns.slice(0, history.length), history, 'Pauses and response changes must preserve session history');
}
live = applyLocalEvent(live, { type: 'transcript', id: '20', text: 'After a pause', final: false });
live = applyLocalEvent(live, { type: 'transcript', id: '20', text: 'After a pause we continue.', final: true });
assert.equal(live.turns.filter(t => t.speaker === 'REMOTE').length, 21);
assert.deepEqual(live.turns.slice(0, history.length), history);
assert.equal(live.turns.at(-1).text, 'After a pause we continue.');
assert.equal(emptyMeeting.turns.length, 0);
assert.equal(applyLocalEvent(live, { type: 'error' }).say.state, 'ERROR');
assert.equal(applyLocalEvent(emptyMeeting, { type: 'say', state: 'STALE' }).say.state, 'IDLE');
console.log('Local event checks passed: partial/final replacement, full session history across pauses, stale suggestions, errors.');
assert.equal(applyLocalEvent({ turns: [], say: { state: 'THINKING' } }, { type: 'status', status: 'Stopped' }).say.state, 'IDLE');
assert.equal(applyLocalEvent({ turns: [], say: { state: 'READY', text: 'A suggestion' } }, { type: 'status', status: 'Stopped' }).say.state, 'STALE');

const options = [
  { kind: 'question', text: 'Which part should we test first?' },
  { kind: 'agreement', text: 'A small pilot makes sense before we expand.' },
  { kind: 'idea', text: 'We could compare two different camera positions.' },
  { kind: 'next_step', text: 'Let’s agree on what the pilot should measure.' },
];
const choices = applyLocalEvent(emptyMeeting, { type: 'say', state: 'READY', text: options.map(o => o.text).join('\n'), options });
assert.deepEqual(choices.say.options, options);
assert.deepEqual(applyLocalEvent(choices, { type: 'say', state: 'STALE' }).say.options, options);
assert.equal(applyLocalEvent(choices, { type: 'say', state: 'THINKING' }).say.options, undefined);
assert.equal(mockAt(12).say.options.length, 4);
console.log('Response choices passed: four kinds, ordering, stale preservation and replacement.');

const { markSaid, restoreMeeting } = await import('../src/ui/meeting/local.ts');
let meeting = applyLocalEvent(emptyMeeting, { type: 'transcript', id: 'capture1:0', text: 'First question?', final: true });
meeting = applyLocalEvent(meeting, { type: 'say', state: 'READY', id: 'reply1', context_id: 'capture1:0', text: 'Options', options });
meeting = applyLocalEvent(meeting, { type: 'status', status: 'Paused' });
meeting = applyLocalEvent(meeting, { type: 'say', state: 'THINKING' });
meeting = applyLocalEvent(meeting, { type: 'say', state: 'STALE' });
meeting = applyLocalEvent(meeting, { type: 'transcript', id: 'capture1:1', text: 'Continuing after a pause.', final: true });
meeting = applyLocalEvent(meeting, { type: 'say', state: 'READY', id: 'reply2', text: 'More options', options });
assert.deepEqual(meeting.turns.map(t => t.id), ['capture1:0', 'reply1', 'capture1:1', 'reply2']);
assert.equal(applyLocalEvent(meeting, { type: 'say', state: 'READY', id: 'reply2', text: 'More options', options }).turns.length, 4);
meeting = markSaid(meeting, 'reply1', 'idea');
assert.equal(meeting.turns[1].saidKind, 'idea');
assert.equal(meeting.turns[1].contextId, 'capture1:0');
assert.equal(JSON.stringify(restoreMeeting(JSON.stringify(meeting)).turns), JSON.stringify(meeting.turns));
assert.equal(markSaid(meeting, 'reply1', 'idea').turns[1].saidKind, undefined);
assert.throws(() => restoreMeeting('{"turns":[{"text":"bad"}]}'));
console.log('Meeting history passed: pause/resume, retained replies, deduplication, manual said marks and storage round trip.');
