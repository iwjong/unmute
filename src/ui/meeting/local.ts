import type { MeetingState, ResponseOption } from './mock';
export type LocalEvent = { type: string; status?: string; id?: string; text?: string; final?: boolean; state?: MeetingState['say']['state']; message?: string; options?: ResponseOption[] };
export const emptyMeeting: MeetingState = { turns: [], say: { state: 'IDLE' } };
export function applyLocalEvent(state: MeetingState, event: LocalEvent): MeetingState {
  if (event.type === 'transcript' && event.id && event.text) {
    const turn = { id: event.id, text: event.text, speaker: 'REMOTE' as const, status: event.final ? 'FINAL' as const : 'PARTIAL' as const };
    const turns = [...state.turns];
    const index = turns.findIndex(t => t.id === turn.id);
    if (index < 0) turns.push(turn); else turns[index] = turn;
    return { ...state, turns };
  }
  if (event.type === 'say' && event.state) {
    if (event.state === 'STALE') return { ...state, say: state.say.text ? { ...state.say, state: 'STALE' } : { state: 'IDLE' } };
    return { ...state, say: { state: event.state, text: event.text, options: event.options } };
  }
  if (event.type === 'status' && event.status === 'Stopped') return { ...state, say: state.say.text ? { ...state.say, state: 'STALE' } : { state: state.say.state === 'ERROR' ? 'ERROR' : 'IDLE' } };
  if (event.type === 'error') return { ...state, say: { state: 'ERROR' } };
  return state;
}
