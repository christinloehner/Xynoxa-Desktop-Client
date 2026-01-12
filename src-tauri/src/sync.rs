use crate::api::XynoxaClient;
use crate::db::{Database, FileRecord};
use notify::{RecursiveMode, Result as NotifyResult, Watcher};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

#[allow(dead_code)]
pub struct SyncHandle {
    sender: Sender<SyncCommand>,
    local_root: PathBuf,
}

impl SyncHandle {
    pub fn new(token: String, local_root: PathBuf, api_url: Option<String>) -> Self {
        let (tx, rx) = channel();

        let worker_token = token.clone();
        let worker_root = local_root.clone();
        let worker_url = api_url.clone();

        // Ensure root exists before watching
        if let Err(e) = fs::create_dir_all(&local_root) {
            log::error!("Failed to create sync root {:?}: {}", local_root, e);
        }

        // Channel for watcher to communicate with worker
        // Actually, easiest is to pipe watcher events to the SAME channel 'tx'.
        // But 'tx' sends SyncCommand. Watcher sends Result<notify::Event>.
        // We need a middleman or closure.

        let tx_for_watcher = tx.clone();
        let worker_root_clone_for_watcher = local_root.clone();

        // Shared flag to suppress watcher events during active sync
        // This prevents the debounce timer from being reset by sync-created files
        let sync_active = Arc::new(AtomicBool::new(false));
        let sync_active_for_watcher = Arc::clone(&sync_active);

        let mut watcher =
            notify::recommended_watcher(move |res: NotifyResult<notify::Event>| match res {
                Ok(event) => {
                    // Skip all events while sync is in progress (prevents debounce reset)
                    if sync_active_for_watcher.load(Ordering::Relaxed) {
                        return;
                    }

                    // Ignore read-only access events
                    if let notify::EventKind::Access(_) = event.kind {
                        return;
                    }

                    log::debug!("Watcher Event: {:?}", event);

                        // Filter out .xynoxa.db/.xynoxa.db, hidden files, and the root directory itself
                    let is_relevant = event.paths.iter().any(|p| {
                        // Ignore the root path itself (we only care about children)
                        if p == &worker_root_clone_for_watcher {
                            return false;
                        }

                        // Check every component to ensure no parent is ignored (specifically .git)
                        if let Ok(rel) = p.strip_prefix(&worker_root_clone_for_watcher) {
                            for component in rel.components() {
                                if let Some(os_str) = component.as_os_str().to_str() {
                                    if os_str == ".git"
                                        || os_str == "node_modules"
                                        || os_str == ".xynoxa.db"
                                        || os_str == ".xynoxa.db"
                                    {
                                        return false;
                                    }
                                }
                            }
                            true
                        } else {
                            false
                        }
                    });

                    if is_relevant {
                        log::info!("FS Event triggered by relevant paths: {:?}", event.paths);
                        let _ = tx_for_watcher.send(SyncCommand::FileSystemEvent(event));
                    } else {
                        log::debug!("FS Event ignored (hidden/irrelevant): {:?}", event.paths);
                    }
                }
                Err(e) => println!("Watch error: {:?}", e),
            })
            .expect("Failed to create watcher");

        watcher
            .watch(&local_root, RecursiveMode::Recursive)
            .expect("Failed to watch root");

        thread::spawn(move || {
            // Worker takes ownership of watcher to keep it alive?
            // Or Handle keeps watcher?
            // If Handle drops, watcher drops.
            // If we move watcher to thread, it stays alive as long as thread.
            // Let's move watcher to worker.

            let mut worker = SyncWorker::new(
                worker_token,
                worker_root,
                worker_url,
                rx,
                Some(Box::new(watcher)),
                sync_active,
            );
            if let Err(e) = worker.run() {
                log::error!("Sync Worker crashed: {}", e);
            }
        });

        Self {
            sender: tx,
            local_root,
        }
    }

    pub fn list_files(&self) -> Result<Vec<FileRecord>, String> {
        let db_path = resolve_db_path(&self.local_root);
        let db = Database::new(&db_path).map_err(|e| e.to_string())?;
        db.get_all_files().map_err(|e| e.to_string())
    }
}

#[allow(dead_code)]
enum SyncCommand {
    ForceSync,
    FileSystemEvent(notify::Event),
}

struct SyncWorker {
    client: XynoxaClient,
    local_root: PathBuf,
    db: Database,
    receiver: Receiver<SyncCommand>,
    #[allow(dead_code)] // Watcher is kept alive by being held here
    watcher: Option<Box<dyn Watcher + Send>>,
    sync_active: Arc<AtomicBool>,
    runtime: tokio::runtime::Runtime,
}

impl SyncWorker {
    fn new(
        token: String,
        local_root: PathBuf,
        api_url: Option<String>,
        receiver: Receiver<SyncCommand>,
        watcher: Option<Box<dyn Watcher + Send>>,
        sync_active: Arc<AtomicBool>,
    ) -> Self {
        // Create DB
        let db_path = resolve_db_path(&local_root);
        let _ = fs::create_dir_all(&local_root);
        let db = Database::new(&db_path).expect("Failed to initialize database");

        // Create reusable runtime - avoids expensive runtime creation on every sync
        let runtime = tokio::runtime::Runtime::new().expect("Failed to create Tokio runtime");

        Self {
            client: XynoxaClient::new(token, api_url.unwrap_or_default()),
            local_root,
            db,
            receiver,
            watcher,
            sync_active,
            runtime,
        }
    }

    #[allow(unused_assignments)] // sync_in_progress IS read in next loop iteration
    fn run(&mut self) -> Result<(), String> {
        log::info!("Sync Worker started.");

        // Initial Sync - suppress watcher events during initial sync
        self.sync_active.store(true, Ordering::Relaxed);
        if let Err(e) = self.scan_and_sync(true) {
            // Full sync on startup
            log::error!("Initial sync failed: {}", e);
        }
        self.sync_active.store(false, Ordering::Relaxed);

        // Debounce configuration: wait 4 seconds after last FS event before syncing
        const DEBOUNCE_DURATION: Duration = Duration::from_secs(4);
        const PERIODIC_SYNC_INTERVAL: Duration = Duration::from_secs(20); // Check for server changes

        let mut last_fs_event: Option<std::time::Instant> = None;
        let mut pending_sync = false;

        loop {
            // Calculate timeout: if we have pending events, use remaining debounce time
            // Otherwise, use periodic sync interval
            let timeout = if pending_sync {
                if let Some(last_event) = last_fs_event {
                    let elapsed = last_event.elapsed();
                    if elapsed >= DEBOUNCE_DURATION {
                        // Debounce period passed, sync now
                        Duration::from_millis(0)
                    } else {
                        DEBOUNCE_DURATION - elapsed
                    }
                } else {
                    DEBOUNCE_DURATION
                }
            } else {
                PERIODIC_SYNC_INTERVAL
            };

            match self.receiver.recv_timeout(timeout) {
                Ok(cmd) => match cmd {
                    SyncCommand::ForceSync => {
                        log::info!("Force sync requested");
                        pending_sync = false;
                        last_fs_event = None;
                        self.sync_active.store(true, Ordering::Relaxed);
                        if let Err(e) = self.scan_and_sync(true) {
                            // Full sync
                            log::error!("Force sync failed: {}", e);
                        }
                        self.sync_active.store(false, Ordering::Relaxed);
                    }
                    SyncCommand::FileSystemEvent(_event) => {
                        // FS events during sync are already filtered by the watcher
                        // Reset debounce timer on each FS event
                        last_fs_event = Some(std::time::Instant::now());
                        pending_sync = true;
                        log::debug!("FS Event received, debounce timer reset (4s)");
                    }
                },
                Err(RecvTimeoutError::Timeout) => {
                    if pending_sync {
                        // Debounce period completed, now sync
                        log::info!("Debounce complete (4s), starting sync...");
                        pending_sync = false;
                        last_fs_event = None;
                        self.sync_active.store(true, Ordering::Relaxed);
                        if let Err(e) = self.scan_and_sync(true) {
                            // Has local changes
                            log::error!("Event sync failed: {}", e);
                        }
                        self.sync_active.store(false, Ordering::Relaxed);
                    } else {
                        // Periodic sync - only pull, no local scan
                        log::debug!("Periodic sync check");
                        self.sync_active.store(true, Ordering::Relaxed);
                        if let Err(e) = self.scan_and_sync(false) {
                            // No local changes
                            log::error!("Periodic sync failed: {}", e);
                        }
                        self.sync_active.store(false, Ordering::Relaxed);
                    }
                }
                Err(RecvTimeoutError::Disconnected) => {
                    log::info!("Channel disconnected. Worker stopping.");
                    break;
                }
            }
        }
        Ok(())
    }

    fn scan_and_sync(&self, has_local_changes: bool) -> Result<(), String> {
        log::debug!("Sync check starting...");

        self.runtime.block_on(async {
            // A. PULL Phase (Server -> Client)
            // Loop until all server events are processed
            let mut processed_any = false;
            loop {
                let cursor = self.db.get_cursor().map_err(|e| e.to_string())?;
                log::debug!("Checking for changes from cursor: {}", cursor);

                let sync_response = self
                    .client
                    .sync_pull(cursor)
                    .await
                    .map_err(|e| e.to_string())?;

                // If no events, we're done with PULL phase
                if sync_response.events.is_empty() {
                    if processed_any {
                        log::info!("All server events processed.");
                    }
                    break;
                }

                processed_any = true;

                log::info!("Processing {} events...", sync_response.events.len());

                for event in sync_response.events {
                    log::info!(
                        "Processing event: {} ({}) for {}",
                        event.id,
                        event.action,
                        event.entity_id
                    );

                    match event.action.as_str() {
                        "create" | "update" | "copy" => {
                            if let Some(data) = event.data {
                                let file_id = event.entity_id.clone();

                                // Determine effective path
                                // API now provides "path" field for ALL entity types (files AND folders)
                                let effective_path_str = if let Some(p) = &data.path {
                                    // Primary path source - server provides full path for all entities
                                    p.clone()
                                } else if let Some(sp) = &data.storage_path {
                                    // Fallback: strip owner prefix if available
                                    if let Some(owner) = &event.owner_id {
                                        let prefix = format!("{}/", owner);
                                        sp.strip_prefix(&prefix).unwrap_or(sp).to_string()
                                    } else {
                                        sp.to_string()
                                    }
                                } else {
                                    // Last resort: use name only (for backward compatibility)
                                    data.name.clone().unwrap_or_default()
                                };

                                if effective_path_str.is_empty() {
                                    continue;
                                }

                                let local_path = self.local_root.join(&effective_path_str);



                                if event.entity_type == "folder" || event.entity_type == "group" || event.entity_type == "group_folder" {
                                    log::info!("Creating folder (type: {}): {}", event.entity_type, effective_path_str);
                                    if let Err(e) = fs::create_dir_all(&local_path) {
                                        log::error!("Failed to create folder {}: {}", effective_path_str, e);
                                    }
                                    let is_group_root = data
                                        .group_folder_id
                                        .as_deref()
                                        .map(|g| g == event.entity_id)
                                        .unwrap_or(false)
                                        && data.parent_id.is_none();
                                    // Track in DB so we can find it by ID later (e.g. for delete)
                                    self.db.insert_or_update(&FileRecord {
                                        path: effective_path_str.clone(),
                                        id: Some(file_id),
                                        hash: "directory".to_string(),
                                        modified_at: 0,
                                        server_version: 0,
                                        group_folder_id: data.group_folder_id.clone(),
                                        is_group_root,
                                    }).map_err(|e| e.to_string())?;
                                } else if event.entity_type == "file" {
                                    let remote_hash = data.hash.unwrap_or_default();

                                    // Check local
                                    let local_hash = compute_hash(&local_path).unwrap_or_default();

                                    if local_hash != remote_hash {
                                        // Need to download
                                        if local_hash.is_empty() {
                                            log::info!("New file from server: {}", effective_path_str);
                                            if let Err(e) = self.download_file(&file_id, &effective_path_str).await {
                                                log::error!("Download failed for {}: {}", effective_path_str, e);
                                            }
                                        } else {
                                            // Conflict check: file exists locally WITH different hash
                                            // Basic strategy: Server wins (for now)
                                            let local_mtime = local_path
                                                .metadata()
                                                .ok()
                                                .and_then(|m| m.modified().ok())
                                                .and_then(|t| {
                                                    t.duration_since(std::time::UNIX_EPOCH).ok()
                                                })
                                                .map(|d| d.as_secs() as i64)
                                                .unwrap_or(0);

                                            let db_rec =
                                                self.db.get_file(&effective_path_str).unwrap_or(None);
                                            let db_mtime = db_rec.as_ref().map(|r| r.modified_at).unwrap_or(0);

                                            if local_mtime > db_mtime {
                                                // Local is newer: conflict. For now, backup and overwrite
                                                log::warn!(
                                                    "Conflict detected for {}. Local newer. Backing up...",
                                                    effective_path_str
                                                );
                                                let backup_path =
                                                    local_path.with_extension("conflict_backup");
                                                let _ = fs::rename(&local_path, &backup_path);
                                                if let Err(e) = self.download_file(&file_id, &effective_path_str).await {
                                                    log::error!("Download failed for {}: {}", effective_path_str, e);
                                                }
                                            } else {
                                                log::info!("Downloading updated content for {}", effective_path_str);
                                                match self.download_file(&file_id, &effective_path_str).await {
                                                    Ok(_) => log::info!("Download complete for {}", effective_path_str),
                                                    Err(e) => {
                                                        log::error!("Download failed for {}: {}", effective_path_str, e)
                                                    }
                                                }
                                            }
                                        }
                                    } else {
                                        // Update DB with correct metadata
                                        self.db
                                            .insert_or_update(&FileRecord {
                                                path: effective_path_str.clone(),
                                                id: Some(file_id),
                                                hash: remote_hash,
                                                modified_at: 0,
                                                server_version: 0,
                                                group_folder_id: data.group_folder_id.clone(),
                                                is_group_root: false,
                                            })
                                            .map_err(|e| e.to_string())?;
                                    }
                                }
                            }
                        }
                        "delete" => {
                            if let Some(record) =
                                self.db.get_file_by_id(&event.entity_id).unwrap_or(None)
                            {
                                log::info!("Deleting local: {}", record.path);
                                let full_path = self.local_root.join(&record.path);

                                // Check if it's a directory
                                if full_path.is_dir() {
                                    if let Err(e) = fs::remove_dir_all(&full_path) {
                                         log::error!("Failed to remove directory {}: {}", record.path, e);
                                    }
                                } else {
                                    if let Err(e) = fs::remove_file(&full_path) {
                                        log::error!("Failed to remove file {}: {}", record.path, e);
                                    }
                                }
                                // Cleanup DB
                                let _ = self.db.delete_file(&record.path);
                            }
                        }
                        "move" => {
                            if let Some(data) = event.data {
                                let file_id = event.entity_id.clone();
                                // Determine new path (reuse logic)
                                let new_path_str = if let Some(p) = &data.path {
                                    p.clone()
                                } else if let Some(sp) = &data.storage_path {
                                    if let Some(owner) = &event.owner_id {
                                        let prefix = format!("{}/", owner);
                                        sp.strip_prefix(&prefix).unwrap_or(sp).to_string()
                                    } else {
                                        sp.to_string()
                                    }
                                } else {
                                    data.name.clone().unwrap_or_default()
                                };

                                if new_path_str.is_empty() {
                                    continue;
                                }

                                // 1. Find old path in DB by ID
                                let old_record_opt = self.db.get_file_by_id(&file_id).unwrap_or(None);

                                if let Some(old_record) = old_record_opt {
                                    let old_local = self.local_root.join(&old_record.path);
                                    let new_local = self.local_root.join(&new_path_str);

                                    log::info!("Moving {} -> {}", old_record.path, new_path_str);

                                    // Ensure parent dirs exist
                                    if let Some(parent) = new_local.parent() {
                                        let _ = fs::create_dir_all(parent);
                                    }

                                    // Actually move
                                    if let Err(e) = fs::rename(&old_local, &new_local) {
                                        log::warn!("Move failed ({}). Falling back to download.", e);
                                        // Fallback: delete old, download new
                                        if let Err(e) = self.download_file(&file_id, &new_path_str).await {
                                            log::error!("Move fallback failed: {}", e);
                                        } else {
                                            // If download worked, remove old file if it still exists
                                            let _ = fs::remove_file(old_local);
                                            let _ = self.db.delete_file(&old_record.path);
                                        }
                                    } else {
                                        // Move succeeded: Verify file integrity
                                        let new_hash = compute_hash(&new_local).unwrap_or_default();
                                        let expected_hash = data.hash.as_deref().unwrap_or(&old_record.hash);
                                        
                                        // Check if file is corrupted (0 bytes or wrong hash)
                                        let metadata = new_local.metadata().ok();
                                        let file_size = metadata.as_ref().map(|m| m.len()).unwrap_or(0);
                                        
                                        if file_size == 0 || (new_hash != expected_hash && !expected_hash.is_empty()) {
                                            log::warn!(
                                                "Move corrupted file {} (size: {}, hash mismatch: {}). Re-downloading...",
                                                new_path_str,
                                                file_size,
                                                new_hash != expected_hash
                                            );
                                            
                                            // Remove corrupted file and download fresh copy
                                            let _ = fs::remove_file(&new_local);
                                            let _ = self.db.delete_file(&old_record.path);
                                            
                                            if let Err(e) = self.download_file(&file_id, &new_path_str).await {
                                                log::error!("Re-download after corrupted move failed: {}", e);
                                            }
                                        } else {
                                            // Move succeeded and file is intact: Update DB with verified hash
                                            let _ = self.db.delete_file(&old_record.path);
                                            let is_group_root = data
                                                .group_folder_id
                                                .as_deref()
                                                .map(|g| g == file_id)
                                                .unwrap_or(false)
                                                && data.parent_id.is_none();
                                            
                                            let modified = metadata
                                                .and_then(|m| m.modified().ok())
                                                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                                                .map(|d| d.as_secs() as i64)
                                                .unwrap_or(old_record.modified_at);
                                            
                                            self.db
                                                .insert_or_update(&FileRecord {
                                                    path: new_path_str.clone(),
                                                    id: Some(file_id),
                                                    hash: new_hash, // Use newly computed hash!
                                                    modified_at: modified,
                                                    server_version: old_record.server_version,
                                                    group_folder_id: data.group_folder_id.clone(),
                                                    is_group_root,
                                                })
                                                .map_err(|e| e.to_string())?;
                                            
                                            log::info!("Move completed successfully: {} -> {}", old_record.path, new_path_str);
                                        }
                                    }
                                } else {
                                    // Not found in DB? Treat as new download (create)
                                    log::warn!(
                                        "Move event for unknown file {}. Treating as create.",
                                        file_id
                                    );
                                    if let Err(e) = self.download_file(&file_id, &new_path_str).await {
                                        log::error!("Move (as create) failed: {}", e);
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }

                // Update Cursor after processing this batch
                if sync_response.next_cursor > cursor {
                    self.db
                        .set_cursor(sync_response.next_cursor)
                        .map_err(|e| e.to_string())?;
                }

                // Continue loop to check for more events
            }

            // B. PUSH Phase (Client -> Server)
            // Skip expensive local scan if no local changes (periodic check only pulls)
            if !has_local_changes {
                log::debug!("Skipping PUSH phase (no local changes)");
                log::debug!("Sync check completed.");
                return Ok(());
            }

            let local_files = self.scan_local_files();
            let db_records = self.db.get_all_files().unwrap_or_default();

            // 1. Check for Deletions
            for db_rec in &db_records {
                if !local_files.contains_key(&db_rec.path) {
                    log::info!("Local delete detected for {}. Pushing...", db_rec.path);

                    if let Some(fid) = &db_rec.id {
                        if db_rec.hash == "directory" {
                            if db_rec.is_group_root {
                                let full_path = self.local_root.join(&db_rec.path);
                                let _ = fs::create_dir_all(&full_path);
                                log::info!("Group root restore: {}", db_rec.path);
                                continue;
                            } else if let Err(e) = self.client.delete_folder(fid).await {
                                log::error!("Failed remote folder delete {}: {}", db_rec.path, e);
                            }
                        } else {
                            if let Err(e) = self.client.soft_delete_file(fid).await {
                                log::error!("Failed remote delete {}: {}", db_rec.path, e);
                            }
                        }
                    }
                    // Always remove from DB if locally gone
                    let _ = self.db.delete_file(&db_rec.path);
                }
            }

            // 2. Check for Updates/Creations
            // Sort keys to ensure parents are processed before children (for folder creation)
            let mut sorted_paths: Vec<String> = local_files.keys().cloned().collect();
            sorted_paths.sort();

            for path in sorted_paths {
                let record = local_files.get(&path).unwrap();
                let db_entry = self.db.get_file(&path).unwrap_or(None);

                if let Some(db_rec) = db_entry {
                    if record.hash != db_rec.hash {
                        // Directories don't change hash in our logic (always "directory")
                        // If we are here, it means either DB has "directory" and local has hash (file now),
                        // or DB has hash (was file) and local is "directory" (now folder).

                        if record.hash == "directory" {
                             log::info!("Local path {} changed from file to folder. Skipping upload (handled as create/move?).", path);
                             // If it changed type, strictly it should be a delete + create.
                             // But for now, just don't crash.
                        } else {
                            log::info!("Local change for {}. Uploading...", path);
                            if let Err(e) = self.upload_file(&path).await {
                                log::error!("Upload failed {}: {}", path, e);
                            }
                        }
                    }
                    if db_rec.id.is_none() {
                        log::warn!("Missing ID for {}. Linking...", path);
                         if record.hash == "directory" {
                            if let Err(e) = self.create_remote_folder(&path).await {
                                log::error!("Folder link failed {}: {}", path, e);
                            }
                        } else {
                            if let Err(e) = self.upload_file(&path).await {
                                log::error!("Link upload failed {}: {}", path, e);
                            }
                        }
                    }
                } else {
                    log::info!("New local item: {}. Creating...", path);
                    if record.hash == "directory" {
                        if let Err(e) = self.create_remote_folder(&path).await {
                            log::error!("New folder creation failed {}: {}", path, e);
                        }
                    } else {
                        if let Err(e) = self.upload_file(&path).await {
                            log::error!("New upload failed {}: {}", path, e);
                        }
                    }
                }
            }

            log::debug!("Sync check completed.");
            Ok::<(), String>(())
        })
    }

    // ... helpers ...

    // ... helpers ...
    fn scan_local_files(&self) -> HashMap<String, FileRecord> {
        let mut files = HashMap::new();

        // Use filter_entry to prevent descending into hidden directories (like .git)
        for entry in WalkDir::new(&self.local_root)
            .into_iter()
            .filter_entry(|e| !is_ignored(e))
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            // Skip root itself
            if path == self.local_root {
                continue;
            }

            let relative = path
                .strip_prefix(&self.local_root)
                .unwrap()
                .to_string_lossy()
                .to_string();

            if entry.file_type().is_file() {
                let existing = self.db.get_file(&relative).unwrap_or(None);
                let hash = compute_hash(path).unwrap_or_default();
                let metadata = path.metadata().unwrap();
                let modified = metadata
                    .modified()
                    .unwrap()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs() as i64;

                files.insert(
                    relative.clone(),
                    FileRecord {
                        path: relative,
                        id: None,
                        hash,
                        modified_at: modified,
                        server_version: 0,
                        group_folder_id: existing.as_ref().and_then(|r| r.group_folder_id.clone()),
                        is_group_root: false,
                    },
                );
            } else if entry.file_type().is_dir() {
                let existing = self.db.get_file(&relative).unwrap_or(None);
                // Track directory
                files.insert(
                    relative.clone(),
                    FileRecord {
                        path: relative,
                        id: None,
                        hash: "directory".to_string(), // Marker
                        modified_at: 0,
                        server_version: 0,
                        group_folder_id: existing.as_ref().and_then(|r| r.group_folder_id.clone()),
                        is_group_root: existing.map(|r| r.is_group_root).unwrap_or(false),
                    },
                );
            }
        }
        files
    }

    async fn download_file(&self, file_id: &str, path: &str) -> Result<(), String> {
        let existing = self.db.get_file_by_id(file_id).unwrap_or(None);
        let mut parent_group_folder_id: Option<String> = None;
        if let Some(parent) = Path::new(path).parent() {
            let parent_str = parent.to_string_lossy();
            if !parent_str.is_empty() && parent_str != "." {
                if let Some(record) = self.db.get_file(&parent_str).unwrap_or(None) {
                    parent_group_folder_id = if record.is_group_root {
                        record.id.clone()
                    } else {
                        record.group_folder_id.clone()
                    };
                }
            }
        }
        let local_path = self.local_root.join(path);
        if let Some(parent) = local_path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }

        self.client.download_file(file_id, &local_path).await?;

        let hash = compute_hash(&local_path).unwrap_or_default();
        let metadata = local_path.metadata().map_err(|e| e.to_string())?;
        let modified = metadata
            .modified()
            .unwrap()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        self.db
            .insert_or_update(&FileRecord {
                path: path.to_string(),
                id: Some(file_id.to_string()),
                hash,
                modified_at: modified,
                server_version: 0,
                group_folder_id: existing
                    .as_ref()
                    .and_then(|r| r.group_folder_id.clone())
                    .or(parent_group_folder_id),
                is_group_root: false,
            })
            .map_err(|e| e.to_string())?;

        Ok(())
    }

    async fn create_remote_folder(&self, path: &str) -> Result<(), String> {
        let relative_path = Path::new(path);
        let name = relative_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        // Find parent ID
        let mut parent_group_folder_id: Option<String> = None;
        let parent_id = if let Some(parent) = relative_path.parent() {
            let parent_str = parent.to_string_lossy();
            if parent_str.len() > 0 && parent_str != "." {
                if let Some(record) = self.db.get_file(&parent_str).unwrap_or(None) {
                    parent_group_folder_id = if record.is_group_root {
                        record.id.clone()
                    } else {
                        record.group_folder_id.clone()
                    };
                    record.id
                } else {
                    let msg = format!(
                        "Parent {} not found for {}. Skipping to prevent flattening.",
                        parent_str, path
                    );
                    log::warn!("{}", msg);
                    return Err(msg);
                }
            } else {
                None
            }
        } else {
            None
        };

        log::info!("Creating remote folder: {} (Parent: {:?})", name, parent_id);

        match self.client.create_folder(&name, parent_id.as_deref()).await {
            Ok(entry) => {
                let group_folder_id = parent_group_folder_id.clone();
                self.db
                    .insert_or_update(&FileRecord {
                        path: path.to_string(),
                        id: Some(entry.id),
                        hash: "directory".to_string(),
                        modified_at: 0,
                        server_version: 0, // Folders don't have versions
                        group_folder_id,
                        is_group_root: false,
                    })
                    .map_err(|e| e.to_string())?;
                Ok(())
            }
            Err(e) => {
                log::warn!(
                    "Create folder failed: {}. Attempting to resolve via adoption...",
                    e
                );
                // Fallback: Check if it already exists (Adoption)
                if let Some(existing_id) = self
                    .find_remote_folder_id(&name, parent_id.as_deref())
                    .await?
                {
                    log::info!("Found existing remote folder {}. Adopting...", existing_id);
                    let group_folder_id = parent_group_folder_id.clone();
                    self.db
                        .insert_or_update(&FileRecord {
                            path: path.to_string(),
                            id: Some(existing_id),
                            hash: "directory".to_string(),
                            modified_at: 0,
                            server_version: 0, // Unknown, but 0 is safe
                            group_folder_id,
                            is_group_root: false,
                        })
                        .map_err(|e| e.to_string())?;
                    Ok(())
                } else {
                    Err(e)
                }
            }
        }
    }

    // Helper to find a folder by name/parent by scanning the sync stream (Expensive but robust fallback)
    async fn find_remote_folder_id(
        &self,
        target_name: &str,
        target_parent: Option<&str>,
    ) -> Result<Option<String>, String> {
        // We scan from 0. In production this should be cached or optimized.
        let mut cursor = 0;
        loop {
            let res = self
                .client
                .sync_pull(cursor)
                .await
                .map_err(|e| e.to_string())?;
            if res.events.is_empty() {
                break;
            }

            for event in &res.events {
                // Log everything for debugging
                if let Some(data) = &event.data {
                    if let Some(n) = &data.name {
                        if n == target_name {
                            log::warn!("Adoption Scan: Found match Candidate! ID: {}, Type: {}, Action: {}, Parent: {:?}",
                                event.entity_id, event.entity_type, event.action, data.parent_id);
                        }
                    }
                }

                // We are looking for a folder or group
                if event.entity_type == "folder" || event.entity_type == "group" || event.entity_type == "group_folder" {
                    match event.action.as_str() {
                        "create" | "update" | "copy" => {
                            if let Some(data) = &event.data {
                                // Check Name
                                if let Some(n) = &data.name {
                                    if n == target_name {
                                        // Check Parent
                                        let remote_parent =
                                            data.folder_id.as_deref().or(data.parent_id.as_deref());
                                        if remote_parent == target_parent {
                                            log::info!(
                                                "Adoption Scan: Confirmed Match for {}",
                                                target_name
                                            );
                                            return Ok(Some(event.entity_id.clone()));
                                        } else {
                                            log::warn!("Adoption Scan: Name matched but Parent Match Failed. Local: {:?}, Remote: {:?}", target_parent, remote_parent);
                                        }
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
                cursor = res.next_cursor;
            }
            if cursor == res.next_cursor || res.events.len() < 50 {
                // End of stream
                break;
            }
        }
        Ok(None)
    }

    async fn upload_file(&self, path: &str) -> Result<(), String> {
        let local_path = self.local_root.join(path);

        // Safety check: Never upload directories as files
        if local_path.is_dir() {
            log::warn!("upload_file called on directory: {}. Skipping.", path);
            return Ok(());
        }

        let existing_record = self.db.get_file(path).unwrap_or(None);
        let existing_id = existing_record.as_ref().and_then(|r| r.id.clone());

        // Determine parent folder ID for proper server-side placement
        let mut parent_group_folder_id: Option<String> = None;
        let parent_folder_id = if let Some(parent) = std::path::Path::new(path).parent() {
            let parent_str = parent.to_string_lossy();
            if !parent_str.is_empty() && parent_str != "." {
                if let Some(record) = self.db.get_file(&parent_str).unwrap_or(None) {
                    parent_group_folder_id = if record.is_group_root {
                        record.id.clone()
                    } else {
                        record.group_folder_id.clone()
                    };
                    record.id
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        let entry = self
            .client
            .upload_file(
                &local_path,
                existing_id.as_deref(),
                parent_folder_id.as_deref(),
                path,
            )
            .await?;

        let hash = compute_hash(&local_path).unwrap_or_default();
        let metadata = local_path.metadata().map_err(|e| e.to_string())?;
        let modified = metadata
            .modified()
            .unwrap()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        self.db
            .insert_or_update(&FileRecord {
                path: path.to_string(),
                id: Some(entry.id),
                hash,
                modified_at: modified,
                server_version: 0, // UploadedFile doesn't have version
                group_folder_id: parent_group_folder_id,
                is_group_root: false,
            })
            .map_err(|e| e.to_string())?;

        Ok(())
    }
}

fn resolve_db_path(local_root: &Path) -> PathBuf {
    let new_path = local_root.join(".xynoxa.db");
    if new_path.exists() {
        return new_path;
    }
    let legacy_path = local_root.join(".xynoxa.db");
    if legacy_path.exists() {
        if fs::rename(&legacy_path, &new_path).is_ok() {
            return new_path;
        }
        return legacy_path;
    }
    new_path
}

fn compute_hash(path: &Path) -> Result<String, String> {
    let mut file = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher).map_err(|e| e.to_string())?;
    Ok(hex::encode(hasher.finalize()))
}

fn is_ignored(entry: &walkdir::DirEntry) -> bool {
    entry
        .file_name()
        .to_str()
        .map(|s| s == ".git" || s == "node_modules" || s == ".xynoxa.db" || s == ".xynoxa.db")
        .unwrap_or(false)
}
