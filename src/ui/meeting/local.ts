import type { MeetingState, ResponseOption } from './mock';
export type LocalEvent = { type: string; status?: string; id?: string; text?: string; final?: boolean; state?: MeetingState['say']['state']; message?: string; options?: ResponseOption[]; context_id?: string };
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
    if (event.state === 'READY' && event.text) {
      const id = event.id ?? `reply-${state.turns.length}`;
      if (state.turns.some(t => t.id === id)) return state;
      return { turns: [...state.turns, { id, speaker: 'YOU', text: event.text, status: 'FINAL', options: event.options, contextId: event.context_id }],
        say: { state: 'READY', text: event.text, options: event.options } };
    }
    if (event.state === 'STALE') return { ...state, say: state.say.text ? { ...state.say, state: 'STALE' } : { state: 'IDLE' } };
    return { ...state, say: { state: event.state, text: event.text, options: event.options } };
  }
  if (event.type === 'status' && event.status === 'Stopped') return { ...state, say: state.say.text ? { ...state.say, state: 'STALE' } : { state: state.say.state === 'ERROR' ? 'ERROR' : 'IDLE' } };
  if (event.type === 'error') return { ...state, say: { state: 'ERROR' } };
  return state;
}

export function markSaid(state: MeetingState, id: string, kind: ResponseOption['kind']): MeetingState {
  return { ...state, turns: state.turns.map(t => t.id === id && t.options?.some(o => o.kind === kind)
    ? { ...t, saidKind: t.saidKind === kind ? undefined : kind } : t) };
}

// Only our own versioned session format is restored; broken storage must not crash startup.
export function restoreMeeting(raw: string | null): MeetingState {
  if (!raw) return emptyMeeting;
  const value = JSON.parse(raw);
  if (!Array.isArray(value.turns) || !value.turns.every((t: any) =>
    typeof t.id === 'string' && typeof t.text === 'string' &&
    ['REMOTE', 'YOU'].includes(t.speaker) && ['PARTIAL', 'FINAL'].includes(t.status) &&
    (t.options === undefined || (Array.isArray(t.options) && t.options.every((o: any) =>
      ['question', 'agreement', 'idea', 'next_step'].includes(o.kind) && typeof o.text === 'string'))))) {
    throw new Error('Saved meeting could not be read.');
  }
  return { turns: value.turns, say: { state: 'IDLE' } };
}
