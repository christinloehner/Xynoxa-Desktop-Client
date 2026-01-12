use reqwest::Client;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::path::Path;
use tokio::fs::File;
use tokio::io::AsyncReadExt;

const MAX_UPLOAD_BYTES: u64 = 5 * 1024 * 1024 * 1024; // 5 GB
const CHUNK_THRESHOLD_BYTES: u64 = 50 * 1024 * 1024; // 50 MB
const CHUNK_SIZE_BYTES: usize = 1 * 1024 * 1024; // 1 MB (align with web uploader; avoid proxy body limits)

#[derive(Clone)]
pub struct XynoxaClient {
    client: Client,
    token: String,
    base_url: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SyncEvent {
    pub id: u64,
    #[serde(rename = "ownerId")]
    pub owner_id: Option<String>,
    pub action: String,
    #[serde(rename = "entityType")]
    pub entity_type: String,
    #[serde(rename = "entityId")]
    pub entity_id: String,
    pub data: Option<FileData>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct FileData {
    pub path: Option<String>, // Often just the filename for files
    pub name: Option<String>, // Name for folders
    #[serde(rename = "storagePath")]
    pub storage_path: Option<String>, // Full path with owner prefix
    #[serde(rename = "folderId")]
    pub folder_id: Option<String>,
    #[serde(rename = "groupFolderId")]
    pub group_folder_id: Option<String>,
    #[serde(rename = "parentId")]
    pub parent_id: Option<String>,
    pub hash: Option<String>,
    pub size: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SyncResponse {
    pub events: Vec<SyncEvent>,
    #[serde(rename = "nextCursor")]
    pub next_cursor: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct FileEntry {
    pub id: String,
    pub name: String,
    pub version: i64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct FolderEntry {
    pub id: String,
    pub name: Option<String>,
}

// Upload API response wrapper: { file: { ... } }
#[derive(Deserialize, Debug, Clone)]
pub struct UploadResponse {
    pub file: UploadedFile,
}

#[derive(Deserialize, Debug, Clone)]
pub struct UploadedFile {
    pub id: String,
    pub path: String,
    pub size: String,
    pub mime: String,
    pub hash: String,
    #[serde(rename = "storagePath")]
    pub storage_path: Option<String>,
}

impl XynoxaClient {
    pub fn new(token: String, base_url: String) -> Self {
        // [WARNING] SSL Verification Disabled for Dev/Testing
        let client = Client::builder()
            .danger_accept_invalid_certs(true)
            .build()
            .unwrap_or_else(|_| Client::new());

        Self {
            client,
            token,
            base_url: base_url.trim_end_matches('/').to_string(),
        }
    }

    pub async fn sync_pull(&self, cursor: u64) -> Result<SyncResponse, String> {
        let url = format!("{}/api/trpc/sync.pull", self.base_url);
        // TRPC v10 standard batch format with 'json' wrapper (match mutation structure)
        let input_json = format!(r#"{{"0":{{"json":{{"cursor":{}}}}}}}"#, cursor);

        log::debug!("Request URL: {}", url);
        log::debug!("Request Input: {}", input_json);

        let res = self
            .client
            .get(&url)
            .bearer_auth(&self.token)
            .query(&[("batch", "1"), ("input", &input_json)])
            .send()
            .await
            .map_err(|e| e.to_string())?;

        // Debug: Read raw text first (always)
        let status = res.status();
        let text = res.text().await.map_err(|e| e.to_string())?;
        log::debug!("Response Status: {}", status);
        log::debug!("Response Body: {}", text);

        if !status.is_success() {
            return Err(format!("Sync Pull Error: {}. Body: {}", status, text));
        }

        // Logic: Try to decode as TrpcResult batch first. If that fails or data structure mismatch,
        // fallback to direct structural checks if possible, or strict TRPC error handling.

        #[derive(Deserialize)]
        struct TrpcResult<T> {
            result: TrpcData<T>,
        }
        #[derive(Deserialize)]
        struct TrpcData<T> {
            data: TrpcResponsePayload<T>,
        }
        #[derive(Deserialize)]
        struct TrpcResponsePayload<T> {
            json: T,
        }

        // Try decoding as standar TRPC Batch format
        if let Ok(wrapped) = serde_json::from_str::<Vec<TrpcResult<SyncResponse>>>(&text) {
            if let Some(first) = wrapped.into_iter().next() {
                return Ok(first.result.data.json);
            }
        }

        // Sometimes TRPC (or server proxy) might return just the result data for single queries??
        // Or duplicate wrapping?
        // Let's try to parse the `json` object directly if it exists in a different structure?
        // Actually, previous log showed `Error decoding response body`.
        // If the server returns 400, we handled it. 500 handled.
        // It must be success 200 but shape mismatch.
        // Let's log raw text in verify step if this still fails.
        // For now, let's also try to see if it returned a bare SyncResponse (unlikely for TRPC but possible if mocked).

        if let Ok(direct) = serde_json::from_str::<SyncResponse>(&text) {
            return Ok(direct);
        }

        Err(format!("Failed to decode response. Raw: {}", text))
    }

    async fn trpc_mutation<T: Serialize, R: DeserializeOwned>(
        &self,
        router_procedure: &str,
        input: &T,
    ) -> Result<R, String> {
        let url = format!("{}/api/trpc/{}?batch=1", self.base_url, router_procedure);

        #[derive(Serialize)]
        struct TrpcBatch<T> {
            #[serde(rename = "0")]
            item: TrpcItem<T>,
        }
        #[derive(Serialize)]
        struct TrpcItem<T> {
            json: T,
        }

        let body = TrpcBatch {
            item: TrpcItem { json: input },
        };

        let res = self
            .client
            .post(&url)
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        if !res.status().is_success() {
            let status = res.status();
            let text = res.text().await.unwrap_or_else(|_| "No body".to_string());
            return Err(format!(
                "TRPC Mutation Error {}: {} Body: {}",
                router_procedure, status, text
            ));
        }

        #[derive(Deserialize)]
        struct TrpcResult<R> {
            result: TrpcData<R>,
        }
        #[derive(Deserialize)]
        struct TrpcData<R> {
            data: TrpcPayload<R>,
        }
        #[derive(Deserialize)]
        struct TrpcPayload<R> {
            json: R,
        }

        // TRPC returns an array of results for batch requests
        // Read text first to debug decoding errors
        let text = res.text().await.map_err(|e| e.to_string())?;

        let wrapped: Vec<TrpcResult<R>> = serde_json::from_str(&text)
            .map_err(|e| format!("Failed to decode TRPC response: {}. Body: {}", e, text))?;

        if let Some(first) = wrapped.into_iter().next() {
            Ok(first.result.data.json)
        } else {
            Err("Empty TRPC response".to_string())
        }
    }

    pub async fn soft_delete_file(&self, file_id: &str) -> Result<(), String> {
        #[derive(Serialize)]
        struct Input {
            #[serde(rename = "fileId")]
            file_id: String,
        }
        self.trpc_mutation(
            "files.softDelete",
            &Input {
                file_id: file_id.to_string(),
            },
        )
        .await
    }

    pub async fn rename_file(&self, file_id: &str, new_name: &str) -> Result<(), String> {
        #[derive(Serialize)]
        struct Input {
            id: String,
            name: String,
        }
        self.trpc_mutation(
            "files.rename",
            &Input {
                id: file_id.to_string(),
                name: new_name.to_string(),
            },
        )
        .await
    }

    pub async fn move_file(
        &self,
        file_id: &str,
        new_parent_id: Option<&str>,
    ) -> Result<(), String> {
        #[derive(Serialize)]
        struct Input {
            id: String,
            #[serde(rename = "folderId")]
            folder_id: Option<String>,
        }
        self.trpc_mutation(
            "files.move",
            &Input {
                id: file_id.to_string(),
                folder_id: new_parent_id.map(|s| s.to_string()),
            },
        )
        .await
    }

    pub async fn restore_file(&self, file_id: &str) -> Result<(), String> {
        #[derive(Serialize)]
        struct Input {
            #[serde(rename = "fileId")]
            file_id: String,
        }
        self.trpc_mutation(
            "files.restore",
            &Input {
                file_id: file_id.to_string(),
            },
        )
        .await
    }

    pub async fn permanent_delete_file(&self, file_id: &str) -> Result<(), String> {
        #[derive(Serialize)]
        struct Input {
            #[serde(rename = "fileId")]
            file_id: String,
        }
        self.trpc_mutation(
            "files.permanentDelete",
            &Input {
                file_id: file_id.to_string(),
            },
        )
        .await
    }

    pub async fn delete_folder(&self, folder_id: &str) -> Result<(), String> {
        #[derive(Serialize)]
        struct Input {
            id: String,
        }
        self.trpc_mutation(
            "folders.delete",
            &Input {
                id: folder_id.to_string(),
            },
        )
        .await
    }

    pub async fn create_folder(
        &self,
        name: &str,
        parent_id: Option<&str>,
    ) -> Result<FolderEntry, String> {
        #[derive(Serialize)]
        struct Input {
            name: String,
            #[serde(rename = "parentId")]
            parent_id: Option<String>,
        }
        self.trpc_mutation(
            "folders.create",
            &Input {
                name: name.to_string(),
                parent_id: parent_id.map(|s| s.to_string()),
            },
        )
        .await
    }

    pub async fn upload_file(
        &self,
        local_path: &Path,
        file_id: Option<&str>,
        folder_id: Option<&str>,
        original_name: &str,
    ) -> Result<UploadedFile, String> {
        let metadata = tokio::fs::metadata(local_path)
            .await
            .map_err(|e| e.to_string())?;
        let file_size = metadata.len();

        if file_size > MAX_UPLOAD_BYTES {
            return Err(format!(
                "File too large (max {} bytes).",
                MAX_UPLOAD_BYTES
            ));
        }

        if file_size > CHUNK_THRESHOLD_BYTES {
            return self
                .upload_file_chunked(local_path, file_id, folder_id, original_name, file_size)
                .await;
        }

        // Safety check: Reject directories
        if local_path.is_dir() {
            return Err(format!(
                "Cannot upload directory as file: {}",
                local_path.display()
            ));
        }

        let url = format!("{}/api/upload", self.base_url);

        let mut file = File::open(local_path).await.map_err(|e| e.to_string())?;
        let mut buffer = Vec::new();
        file.read_to_end(&mut buffer)
            .await
            .map_err(|e| e.to_string())?;

        // Detect MIME type from file extension using mime_guess
        let mime_type = mime_guess::from_path(local_path)
            .first_or_octet_stream()
            .to_string();

        log::debug!("Uploading {} with MIME type: {}", original_name, mime_type);

        let body = reqwest::Body::from(buffer);
        let part = reqwest::multipart::Part::stream(body)
            .file_name(original_name.to_string())
            .mime_str(&mime_type)
            .map_err(|e| format!("Invalid MIME type: {}", e))?;

        let mut form = reqwest::multipart::Form::new()
            .part("file", part)
            .text("originalName", original_name.to_string());

        if let Some(fid) = file_id {
            form = form.text("fileId", fid.to_string());
        }

        if let Some(folder) = folder_id {
            form = form.text("folderId", folder.to_string());
        }

        let res = self
            .client
            .post(&url)
            .bearer_auth(&self.token)
            .multipart(form)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        if !res.status().is_success() {
            let status = res.status();
            let body = res.text().await.unwrap_or_else(|_| "No body".to_string());
            return Err(format!("Upload failed: {}. Body: {}", status, body));
        }

        // API returns { file: { ... } } wrapper
        let upload_response: UploadResponse = res.json().await.map_err(|e| e.to_string())?;
        Ok(upload_response.file)
    }

    async fn upload_file_chunked(
        &self,
        local_path: &Path,
        file_id: Option<&str>,
        folder_id: Option<&str>,
        original_name: &str,
        file_size: u64,
    ) -> Result<UploadedFile, String> {
        // Safety check: Reject directories
        if local_path.is_dir() {
            return Err(format!(
                "Cannot upload directory as file: {}",
                local_path.display()
            ));
        }

        let mime_type = mime_guess::from_path(local_path)
            .first_or_octet_stream()
            .to_string();

        let total_chunks = ((file_size as f64) / (CHUNK_SIZE_BYTES as f64)).ceil() as u64;

        #[derive(Serialize)]
        struct StartPayload {
            filename: String,
            #[serde(rename = "originalName")]
            original_name: String,
            size: u64,
            #[serde(rename = "totalChunks")]
            total_chunks: u64,
            mime: String,
            #[serde(rename = "fileId")]
            file_id: Option<String>,
        }

        #[derive(Deserialize)]
        struct StartResponse {
            #[serde(rename = "uploadId")]
            upload_id: String,
        }

        let start_url = format!("{}/api/upload/chunk/start", self.base_url);
        let start_payload = StartPayload {
            filename: original_name.to_string(),
            original_name: original_name.to_string(),
            size: file_size,
            total_chunks,
            mime: mime_type.clone(),
            file_id: file_id.map(|s| s.to_string()),
        };

        let start_res = self
            .client
            .post(&start_url)
            .bearer_auth(&self.token)
            .json(&start_payload)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        if !start_res.status().is_success() {
            let status = start_res.status();
            let text = start_res.text().await.unwrap_or_else(|_| "No body".to_string());
            return Err(format!("Chunk start failed: {}. Body: {}", status, text));
        }

        let start_response: StartResponse = start_res.json().await.map_err(|e| e.to_string())?;
        let upload_id = start_response.upload_id;

        let mut file = File::open(local_path).await.map_err(|e| e.to_string())?;
        let mut chunk_index: u64 = 0;
        let mut buffer = vec![0u8; CHUNK_SIZE_BYTES];

        loop {
            let bytes_read = file
                .read(&mut buffer)
                .await
                .map_err(|e| e.to_string())?;
            if bytes_read == 0 {
                break;
            }

            let chunk = buffer[..bytes_read].to_vec();
            let part = reqwest::multipart::Part::bytes(chunk)
                .file_name(format!("{}.part", chunk_index))
                .mime_str(&mime_type)
                .map_err(|e| e.to_string())?;

            let form = reqwest::multipart::Form::new()
                .text("uploadId", upload_id.clone())
                .text("chunkIndex", chunk_index.to_string())
                .part("file", part);

            let chunk_url = format!("{}/api/upload/chunk", self.base_url);
            let chunk_res = self
                .client
                .post(&chunk_url)
                .bearer_auth(&self.token)
                .multipart(form)
                .send()
                .await
                .map_err(|e| e.to_string())?;

            if !chunk_res.status().is_success() {
                let status = chunk_res.status();
                let text = chunk_res.text().await.unwrap_or_else(|_| "No body".to_string());
                return Err(format!("Chunk upload failed: {}. Body: {}", status, text));
            }

            chunk_index += 1;
        }

        #[derive(Serialize)]
        struct CompletePayload {
            #[serde(rename = "uploadId")]
            upload_id: String,
            #[serde(rename = "folderId")]
            folder_id: Option<String>,
        }

        let complete_url = format!("{}/api/upload/chunk/complete", self.base_url);
        let complete_payload = CompletePayload {
            upload_id,
            folder_id: folder_id.map(|s| s.to_string()),
        };

        let complete_res = self
            .client
            .post(&complete_url)
            .bearer_auth(&self.token)
            .json(&complete_payload)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        if !complete_res.status().is_success() {
            let status = complete_res.status();
            let text = complete_res.text().await.unwrap_or_else(|_| "No body".to_string());
            return Err(format!("Chunk complete failed: {}. Body: {}", status, text));
        }

        let upload_response: UploadResponse = complete_res.json().await.map_err(|e| e.to_string())?;
        Ok(upload_response.file)
    }

    pub async fn download_file(&self, file_id: &str, local_path: &Path) -> Result<(), String> {
        // Use path parameter format - encode file_id for special characters
        let encoded_id = urlencoding::encode(file_id);
        let url = format!("{}/api/files/{}/content", self.base_url, encoded_id);

        let res = self
            .client
            .get(&url)
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        let status = res.status();
        log::debug!("Download Response Status: {}", status);

        if !status.is_success() {
            let body = res.text().await.unwrap_or_else(|_| "No body".to_string());
            log::error!("Download Error Body: {}", body);
            return Err(format!("Download failed: {}. Body: {}", status, body));
        }

        let content = res.bytes().await.map_err(|e| e.to_string())?;

        if let Some(parent) = local_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| e.to_string())?;
        }

        tokio::fs::write(local_path, content)
            .await
            .map_err(|e| e.to_string())?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_file_entry_serialization() {
        let entry = FileEntry {
            id: "123".into(),
            name: "test.txt".into(),
            version: 1,
        };
        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains("test.txt"));
    }
}
