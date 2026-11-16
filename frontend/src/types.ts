export type Location = 'vault' | 'cloud'
export type View = 'files' | 'favorites' | 'offline' | 'activity' | 'trash' | 'settings'
export type Entry = {
  id: string; name: string; parentId: string | null; kind: 'file' | 'folder'; size: number; modified: number
  starred: boolean; pinned: boolean; trashed: boolean; syncState: 'local' | 'uploading' | 'synced'
}
export type AppStatus = {
  vaultExists: boolean; unlocked: boolean; connected: boolean; accountEmail: string | null; region: 'us' | 'eu' | null
  cacheLimitBytes: number; cacheBytes: number; pendingCount: number
  lastSync: number | null; oauthClientId: string; error?: string | null
}
export type Activity = { id: string; message: string; time: number; kind: 'info' | 'success' | 'error' }
export type ActionArguments = {
  create_vault: { password: string; remember: boolean }
  unlock_vault: { password: string; remember: boolean }
  lock_vault: Record<string, never>
  forget_device: Record<string, never>
  connect_pcloud: { clientId: string }
  connect_pcloud_web: Record<string, never>
  save_settings: { cacheLimitGib: number; clientId: string }
  new_folder: { location: Location; parentId: string | null; name: string }
  rename_entry: { location: Location; id: string; name: string }
  move_entry: { location: Location; id: string; parentId: string | null }
  trash_entry: { location: Location; id: string }
  restore_entry: { location: Location; id: string }
  toggle_favorite: { id: string }
  toggle_pin: { id: string }
  import_files: { parentId: string | null }
  import_cloud_file: { id: string; parentId: string | null }
  download_entry: { location: Location; id: string }
  open_entry: { location: Location; id: string }
  sync_now: Record<string, never>
  change_password: { oldPassword: string; newPassword: string }
  export_recovery: Record<string, never>
}
export type ActionResults = {
  create_vault: AppStatus; unlock_vault: AppStatus; lock_vault: AppStatus; forget_device: AppStatus
  connect_pcloud: AppStatus; connect_pcloud_web: AppStatus; save_settings: AppStatus
  new_folder: void; rename_entry: void; move_entry: void; trash_entry: void; restore_entry: void
  toggle_favorite: void; toggle_pin: void; import_files: void; import_cloud_file: void; download_entry: void
  open_entry: void; sync_now: void; change_password: void; export_recovery: void
}

// ---- 0.3.0 contract shared by the UI and src-tauri commands (camelCase over IPC) ----
/** everything: all new uploads/backups/syncs go to the Vault. choose: ask each time. vaultOnly: they go to plain pCloud. */
export type Protection = 'everything' | 'choose' | 'vaultOnly'
export type Theme = 'system' | 'light' | 'dark'
export type Preferences = { protection: Protection | null; theme: Theme; startAtLogin: boolean }
/** A destination: a folder in the Vault (encrypted) or in pCloud (not encrypted). folderId null = root. */
export type Target = { location: Location; folderId: string | null; label: string }
export type JobKind = 'sync' | 'backup'
export type JobState = 'idle' | 'scanning' | 'transferring' | 'paused' | 'waitingForVault' | 'waitingForConnection' | 'needsReview' | 'error'
export type FolderJob = {
  id: string; kind: JobKind; localPath: string; target: Target; paused: boolean; mirrorDeletions: boolean
  lastRun: number | null; state: JobState; message: string | null; pendingDeletes: number; fileCount: number
}
export type Transfer = {
  id: string; name: string; direction: 'upload' | 'download'; location: Location
  done: number; total: number; state: 'queued' | 'active' | 'done' | 'failed'; error: string | null; time: number
}
export type Photo = { id: string; name: string; location: Location; modified: number; size: number }
export type PublicLink = { id: string; entryId: string; name: string; url: string; created: number; downloads: number; isFolder: boolean }
export type Share = {
  id: string; direction: 'incoming' | 'outgoing'; folderName: string; email: string
  permissions: 'view' | 'edit'; state: 'pending' | 'active'; folderId: string | null
}
export type FileRequest = {
  id: string; folderId: string; folderName: string; url: string; comment: string; expires: number | null
  maxFiles: number | null; filesReceived: number; encrypt: boolean; vaultFolderId: string | null; vaultFolderName: string | null
}
export type AccountInfo = { email: string; region: 'us' | 'eu'; usedBytes: number; quotaBytes: number; premium: boolean }
export type KnownFolders = { documents: string | null; pictures: string | null; desktop: string | null }
export type ExtraArguments = {
  set_preferences: Partial<Preferences>
  /** replaces: id of an existing job for the same folder that this one takes over (Switch to encrypted). */
  add_job: { kind: JobKind; localPath: string; location: Location; folderId: string | null; mirrorDeletions: boolean; replaces?: string }
  update_job: { id: string; paused?: boolean; mirrorDeletions?: boolean }
  remove_job: { id: string }
  run_job: { id: string }
  /** apply=true performs the held deletions, false restores/re-uploads instead and resumes. */
  resolve_guard: { id: string; apply: boolean }
  open_recycle: { id: string }
  upload_paths: { paths: string[]; location: Location; folderId: string | null }
  upload_files: { location: Location; folderId: string | null }
  upload_folder: { location: Location; folderId: string | null }
  clear_transfers: Record<string, never>
  create_link: { id: string }
  delete_link: { id: string }
  share_folder: { id: string; email: string; permissions: 'view' | 'edit' }
  accept_share: { id: string }
  decline_share: { id: string }
  remove_share: { id: string }
  create_file_request: { folderId: string; comment: string; expiresDays: number | null; maxFiles: number | null; encrypt: boolean; vaultFolderId: string | null }
  delete_file_request: { id: string }
  open_url: { url: string }
}
export type ExtraResults = {
  set_preferences: Preferences; add_job: FolderJob; update_job: FolderJob; remove_job: void; run_job: void
  resolve_guard: void; open_recycle: void; upload_paths: void; upload_files: void; upload_folder: void
  clear_transfers: void; create_link: PublicLink; delete_link: void; share_folder: void; accept_share: void
  decline_share: void; remove_share: void; create_file_request: FileRequest; delete_file_request: void; open_url: void
}
/** Read-only queries: get_preferences, account_info, known_folders, pick_folder, list_jobs, get_transfers,
 *  list_photos {source: 'all'|'vault'|'cloud'}, photo_thumb {location, id} -> data URL | null,
 *  list_links, list_shares, list_file_requests. */
