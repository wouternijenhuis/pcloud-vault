import type { Location } from './types'

/** Runs one backend call with the shared busy state, error banner and confirmation toast. */
export type Perform = (task: () => Promise<unknown>, success?: string) => Promise<boolean>
/** Anything the preview dialog can show: a file in the Vault, in pCloud, or a photo. */
export type PreviewItem = { location: Location; id: string; name: string; size: number; modified: number }
