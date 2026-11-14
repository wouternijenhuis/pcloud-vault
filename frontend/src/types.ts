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
