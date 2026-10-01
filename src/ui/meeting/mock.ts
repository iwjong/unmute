// Deterministic UX fixtures only. Replace this module with real conversation events at M1/M2.
export type Turn = { id: string; speaker: 'REMOTE' | 'YOU'; text: string; status: 'PARTIAL' | 'FINAL' };
export type SayState = 'IDLE' | 'THINKING' | 'READY' | 'STALE' | 'ERROR';
export type ResponseOption = { kind: 'question' | 'agreement' | 'idea' | 'next_step'; text: string };
export type MeetingState = { turns: Turn[]; say: { state: SayState; text?: string; options?: ResponseOption[] } };
const turn = (id: string, speaker: Turn['speaker'], text: string, status: Turn['status'] = 'FINAL'): Turn => ({ id, speaker, text, status });
const opening = [
  turn('1', 'REMOTE', 'We need to see how someone moves, even when one camera loses the view.'),
  turn('2', 'YOU', 'Right. That’s why we use two cameras.'),
];
const question = turn('3', 'REMOTE', 'And how do you combine the data?');
const answer = 'We combine the two camera views to create a more reliable 3D view of the movement. If one view is blocked, the other helps fill the gap.';
const response = turn('4', 'YOU', 'We combine both views, so we can keep tracking when one camera is blocked.');
const later = [...opening, question, response,
  turn('5', 'REMOTE', 'That makes sense. We can start with a small pilot.'),
  turn('6', 'YOU', 'A pilot would let us test it in your space.'),
  turn('7', 'REMOTE', 'Let me check the dates with the team.'),
];
export const moments: { at: number; label: string; state: MeetingState }[] = [
  { at: 0, label: 'Listening', state: { turns: opening, say: { state: 'IDLE' } } },
  { at: 4, label: 'Partial question', state: { turns: [...opening, { ...question, text: 'And how do you combine…', status: 'PARTIAL' }], say: { state: 'IDLE' } } },
  { at: 8, label: 'Considering a response', state: { turns: [...opening, question], say: { state: 'THINKING' } } },
  { at: 12, label: 'Response ready', state: { turns: [...opening, question], say: { state: 'READY', text: answer, options: [
    { kind: 'question', text: 'Would it help if I walked you through how we combine the views?' },
    { kind: 'agreement', text: 'Yes, combining the views is the key part of the approach.' },
    { kind: 'idea', text: 'We could compare both camera views side by side to make the process clearer.' },
    { kind: 'next_step', text: 'Let’s look at one movement example together.' },
  ] } } },
  { at: 24, label: 'Your partial response', state: { turns: [...opening, question, { ...response, text: 'We combine both views, so we can keep tracking…', status: 'PARTIAL' }], say: { state: 'STALE', text: answer } } },
  { at: 28, label: 'Your final response', state: { turns: [...opening, question, response], say: { state: 'STALE', text: answer } } },
  { at: 36, label: 'Recent context', state: { turns: later, say: { state: 'IDLE' } } },
  { at: 44, label: 'Next question', state: { turns: [...later, turn('8', 'REMOTE', 'What would you need from us to get started?')], say: { state: 'THINKING' } } },
  { at: 48, label: 'Response unavailable', state: { turns: [...later, turn('8', 'REMOTE', 'What would you need from us to get started?')], say: { state: 'ERROR' } } },
  { at: 54, label: 'Response recovered', state: { turns: [...later, turn('8', 'REMOTE', 'What would you need from us to get started?')], say: { state: 'READY', text: 'Let’s agree on what the pilot should measure.', options: [
    { kind: 'question', text: 'What would a successful pilot look like for your team?' },
    { kind: 'agreement', text: 'Starting small sounds like a sensible way to evaluate this together.' },
    { kind: 'idea', text: 'We could choose one representative movement to keep the pilot focused.' },
    { kind: 'next_step', text: 'Let’s agree on what the pilot should measure.' },
  ] } } },
];
export const DEMO_END = 60;
export function mockAt(seconds: number): MeetingState {
  return (moments.filter(moment => moment.at <= seconds).at(-1) ?? moments[0]).state;
}
export function recentTurns(state: MeetingState): Turn[] { return state.turns.slice(-6); }
export function nextMoment(seconds: number): number {
  return moments.find(moment => moment.at > seconds)?.at ?? 0;
}
