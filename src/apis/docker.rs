use actix_web::{delete, get, post, web, HttpRequest, HttpResponse, Responder};
use bollard::Docker;
use bollard::models::{ContainerCreateResponse, HostConfig, PortBinding, Mount, MountTypeEnum, ContainerCreateBody};
use bollard::query_parameters::{CreateContainerOptions, CreateImageOptions, ListImagesOptions, RemoveImageOptions};
use futures_util::stream::StreamExt;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::utils::admin::verify_admin_access;
use crate::utils::consts::{FORBID_DIRECTORY, ZUTI_SETTING_FILE};
use crate::DbPool;

// ============================================================================
// Task Status Management (In-Memory Storage)
// ============================================================================

lazy_static::lazy_static! {
    static ref PULL_TASKS: Arc<Mutex<HashMap<String, PullTask>>> = 
        Arc::new(Mutex::new(HashMap::new()));
}

/// Task Status
#[derive(Serialize, Clone, Debug)]
pub enum TaskStatus {
    Pending,    // Waiting to execute
    Running,    // Executing
    Completed,  // Completed
    Failed,     // Failed
}

/// Layer download status
#[derive(Serialize, Clone, Debug)]
pub struct LayerStatus {
    pub layer_id: String,
    pub state: LayerState,
    pub current: i64,      // Downloaded bytes
    pub total: i64,        // Total bytes
    pub progress: u8,      // 0-100 for this layer
}

#[derive(Serialize, Clone, Debug)]
pub enum LayerState {
    Waiting,      // Waiting to download
    Downloading,  // Currently downloading
    Extracting,   // Extracting layer
    Complete,     // Download and extract done
}

/// Pull Task Information
#[derive(Serialize, Clone)]
pub struct PullTask {
    pub task_id: String,
    pub image_name: String,
    pub status: TaskStatus,
    pub progress: u8,           // 0-100 overall
    pub current_layer: String,  // Current layer being processed
    pub message: String,        // Status description
    pub total_layers: usize,
    pub completed_layers: usize,
    pub layers: Vec<LayerStatus>, // All layers status
    pub created_at: u64,
    pub updated_at: u64,
}

/// Start Task Request
#[derive(Deserialize)]
pub struct StartPullRequest {
    pub image_name: String,
}

/// Start Task Response
#[derive(Serialize)]
pub struct StartPullResponse {
    pub success: bool,
    pub task_id: String,
    pub message: String,
}

/// Get Task Response
#[derive(Serialize)]
pub struct GetTaskResponse {
    pub success: bool,
    pub task: Option<PullTask>,
    pub message: String,
}

/// List Tasks Response
#[derive(Serialize)]
pub struct ListTasksResponse {
    pub success: bool,
    pub tasks: Vec<PullTask>,
    pub message: String,
}

/// Format image ID: remove "sha256:" prefix and truncate to 12 chars
fn format_short_id(id: &str) -> String {
    let without_prefix = id.strip_prefix("sha256:").unwrap_or(id);
    without_prefix.chars().take(12).collect()
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn update_task<F>(task_id: &str, updater: F)
where
    F: FnOnce(&mut PullTask),
{
    if let Ok(mut tasks) = PULL_TASKS.lock() {
        if let Some(task) = tasks.get_mut(task_id) {
            updater(task);
            task.updated_at = now_secs();
        }
    }
}

// ============================================================================
// Docker Image Related Structures
// ============================================================================

#[derive(Serialize, Deserialize)]
pub struct DockerImage {
    pub id: String,
    pub repo_tags: Vec<String>,
}

#[derive(Serialize)]
pub struct GetImagesResponse {
    pub success: bool,
    pub message: String,
    pub images: Vec<DockerImage>,
}

/// Delete image response
#[derive(Serialize)]
pub struct DeleteImageResponse {
    pub success: bool,
    pub message: String,
    pub deleted: Vec<String>,
}

// ============================================================================
// API Endpoints
// ============================================================================

/// Get Docker Image List (Admin Only)
#[get("/docker/get_images")]
pub async fn get_images(req: HttpRequest, pool: web::Data<DbPool>) -> impl Responder {
    let _admin_username = match verify_admin_access(&req, &pool).await {
        Ok(username) => username,
        Err(response) => return response,
    };

    let docker = match Docker::connect_with_local_defaults() {
        Ok(docker) => docker,
        Err(e) => {
            return HttpResponse::InternalServerError().json(GetImagesResponse {
                success: false,
                message: format!("Failed to connect to Docker: {}", e),
                images: vec![],
            });
        }
    };

    let options = Some(ListImagesOptions {
        all: true,
        ..Default::default()
    });

    match docker.list_images(options).await {
        Ok(images) => {
            let docker_images: Vec<DockerImage> = images
                .into_iter()
                .map(|img| DockerImage {
                    id: format_short_id(&img.id),
                    repo_tags: img.repo_tags,
                })
                .collect();

            HttpResponse::Ok().json(GetImagesResponse {
                success: true,
                message: "Images retrieved successfully".to_string(),
                images: docker_images,
            })
        }
        Err(e) => HttpResponse::InternalServerError().json(GetImagesResponse {
            success: false,
            message: format!("Failed to list images: {}", e),
            images: vec![],
        }),
    }
}

/// Delete Docker Image by short ID (Admin Only)
///
/// # Endpoint
/// DELETE /docker/delete_image/{short_id}
///
/// # Path Parameters
/// - `short_id`: First 12 characters of sha256 image ID
///
/// # Response
/// { "success": true, "message": "Image deleted", "deleted": ["sha256:xxx..."] }
#[delete("/docker/delete_image/{short_id}")]
pub async fn delete_image(
    req: HttpRequest,
    pool: web::Data<DbPool>,
    path: web::Path<String>,
) -> impl Responder {
    // 1. Verify admin
    let _admin_username = match verify_admin_access(&req, &pool).await {
        Ok(username) => username,
        Err(response) => return response,
    };

    let short_id = path.into_inner();

    // 2. Validate short_id length
    if short_id.len() != 12 {
        return HttpResponse::BadRequest().json(DeleteImageResponse {
            success: false,
            message: "Invalid image ID format, expected 12 characters".to_string(),
            deleted: vec![],
        });
    }

    // 3. Connect to Docker/Podman
    let docker = match Docker::connect_with_local_defaults() {
        Ok(d) => d,
        Err(e) => {
            return HttpResponse::InternalServerError().json(DeleteImageResponse {
                success: false,
                message: format!("Failed to connect to Docker: {}", e),
                deleted: vec![],
            });
        }
    };

    // 4. Find full image ID by short ID
    let images = match docker.list_images(Some(ListImagesOptions { all: true, ..Default::default() })).await {
        Ok(imgs) => imgs,
        Err(e) => {
            return HttpResponse::InternalServerError().json(DeleteImageResponse {
                success: false,
                message: format!("Failed to list images: {}", e),
                deleted: vec![],
            });
        }
    };

    // Find matching image
    let full_id = images
        .into_iter()
        .find(|img| {
            let img_short_id = format_short_id(&img.id);
            img_short_id == short_id
        })
        .map(|img| img.id);

    let full_id = match full_id {
        Some(id) => id,
        None => {
            return HttpResponse::NotFound().json(DeleteImageResponse {
                success: false,
                message: format!("Image with ID '{}' not found", short_id),
                deleted: vec![],
            });
        }
    };

    // 5. Delete image
    let options = Some(RemoveImageOptions {
        force: false,
        ..Default::default()
    });

    match docker.remove_image(&full_id, options, None).await {
        Ok(deleted_images) => {
            let deleted_ids: Vec<String> = deleted_images
                .into_iter()
                .filter_map(|d| d.deleted.or(d.untagged))
                .collect();

            HttpResponse::Ok().json(DeleteImageResponse {
                success: true,
                message: format!("Image '{}' deleted successfully", short_id),
                deleted: deleted_ids,
            })
        }
        Err(e) => {
            HttpResponse::InternalServerError().json(DeleteImageResponse {
                success: false,
                message: format!("Failed to delete image: {}", e),
                deleted: vec![],
            })
        }
    }
}

/// Start Image Pull Task (Async, Polling)
///
/// # Endpoint
/// POST /docker/pull_image/start
///
/// # Request Body
/// { "image_name": "nginx:latest" }
///
/// # Response
/// { "success": true, "task_id": "uuid", "message": "Task created" }
#[post("/docker/pull_image/start")]
pub async fn start_pull_image(
    req: HttpRequest,
    pool: web::Data<DbPool>,
    body: web::Json<StartPullRequest>,
) -> impl Responder {
    // 1. Verify admin
    let _admin_username = match verify_admin_access(&req, &pool).await {
        Ok(username) => username,
        Err(response) => return response,
    };

    // 2. Create task
    let task_id = uuid::Uuid::new_v4().to_string();
    let now = now_secs();
    let image_name = body.image_name.trim().to_string();

    let task = PullTask {
        task_id: task_id.clone(),
        image_name: image_name.clone(),
        status: TaskStatus::Pending,
        progress: 0,
        current_layer: String::new(),
        message: "Waiting to execute".to_string(),
        total_layers: 0,
        completed_layers: 0,
        layers: Vec::new(),
        created_at: now,
        updated_at: now,
    };

    PULL_TASKS.lock().unwrap().insert(task_id.clone(), task);

    // 3. Start background task
    let task_id_clone = task_id.clone();
    actix_web::rt::spawn(async move {
        execute_pull_task(task_id_clone, image_name).await;
    });

    HttpResponse::Ok().json(StartPullResponse {
        success: true,
        task_id,
        message: "Task created, use task_id to poll for progress".to_string(),
    })
}

/// Get Pull Task Status (Polling API)
///
/// # Endpoint
/// GET /docker/pull_image/task/{task_id}
#[get("/docker/pull_image/task/{task_id}")]
pub async fn get_pull_task(
    req: HttpRequest,
    pool: web::Data<DbPool>,
    path: web::Path<String>,
) -> impl Responder {
    let _admin_username = match verify_admin_access(&req, &pool).await {
        Ok(username) => username,
        Err(response) => return response,
    };

    let task_id = path.into_inner();

    match PULL_TASKS.lock().unwrap().get(&task_id) {
        Some(task) => HttpResponse::Ok().json(GetTaskResponse {
            success: true,
            task: Some(task.clone()),
            message: "Task found".to_string(),
        }),
        None => HttpResponse::NotFound().json(GetTaskResponse {
            success: false,
            task: None,
            message: "Task not found".to_string(),
        }),
    }
}

/// List All Pull Tasks
///
/// # Endpoint
/// GET /docker/pull_image/tasks
#[get("/docker/pull_image/tasks")]
pub async fn list_pull_tasks(req: HttpRequest, pool: web::Data<DbPool>) -> impl Responder {
    let _admin_username = match verify_admin_access(&req, &pool).await {
        Ok(username) => username,
        Err(response) => return response,
    };

    let tasks: Vec<PullTask> = PULL_TASKS
        .lock()
        .unwrap()
        .values()
        .cloned()
        .collect();

    let count = tasks.len();
    HttpResponse::Ok().json(ListTasksResponse {
        success: true,
        tasks,
        message: format!("Found {} tasks", count),
    })
}

// ============================================================================
// Background Task Execution
// ============================================================================

async fn execute_pull_task(task_id: String, image_name: String) {
    // Update status to running
    update_task(&task_id, |t| {
        t.status = TaskStatus::Running;
        t.message = "Connecting to Docker/Podman...".to_string();
        t.progress = 5;
    });

    // Connect to Docker/Podman
    let docker = match Docker::connect_with_local_defaults() {
        Ok(d) => d,
        Err(e) => {
            update_task(&task_id, |t| {
                t.status = TaskStatus::Failed;
                t.message = format!("Connection failed: {}", e);
                t.progress = 0;
            });
            return;
        }
    };

    // Parse image name
    let (repo, tag) = if image_name.contains(':') {
        let parts: Vec<&str> = image_name.splitn(2, ':').collect();
        (parts[0].to_string(), parts[1].to_string())
    } else {
        (image_name.clone(), "latest".to_string())
    };

    update_task(&task_id, |t| {
        t.message = format!("Starting pull {}:{} ...", repo, tag);
        t.progress = 5;
    });

    // Create pull options
    let options = Some(CreateImageOptions {
        from_image: Some(repo),
        tag: Some(tag),
        ..Default::default()
    });

    // Execute pull with layer tracking
    let mut stream = docker.create_image(options, None, None);
    let mut last_status = String::new();

    while let Some(result) = stream.next().await {
        match result {
            Ok(status) => {
                // Update status message
                if let Some(status_str) = &status.status {
                    last_status = status_str.clone();
                }

                // Get layer ID
                let layer_id = status.id.clone().unwrap_or_default();

                // Update layer tracking and calculate overall progress
                update_task(&task_id, |t| {
                    // Find or create layer entry
                    let layer_index = t.layers.iter().position(|l| l.layer_id == layer_id);
                    
                    let layer_state = match status.status.as_deref() {
                        Some(s) if s.contains("Pulling fs layer") => Some(LayerState::Waiting),
                        Some(s) if s.contains("Downloading") => Some(LayerState::Downloading),
                        Some(s) if s.contains("Extracting") => Some(LayerState::Extracting),
                        Some(s) if s.contains("Pull complete") || s.contains("Already exists") => Some(LayerState::Complete),
                        _ => None,
                    };

                    if let Some(state) = layer_state {
                        // Calculate layer progress from progress_detail
                        let (current, total, layer_progress) = status.progress_detail.as_ref()
                            .and_then(|d| {
                                match (d.current, d.total) {
                                    (Some(c), Some(tot)) if tot > 0 => {
                                        let prog = ((c as f64 / tot as f64) * 100.0) as u8;
                                        Some((c, tot, prog))
                                    }
                                    _ => None
                                }
                            })
                            .unwrap_or((0, 0, match state {
                                LayerState::Waiting => 0,
                                LayerState::Downloading => 50,
                                LayerState::Extracting => 90,
                                LayerState::Complete => 100,
                            }));

                        if let Some(index) = layer_index {
                            // Update existing layer
                            t.layers[index].state = state.clone();
                            t.layers[index].current = current;
                            t.layers[index].total = total;
                            t.layers[index].progress = layer_progress;
                        } else if !layer_id.is_empty() {
                            // Add new layer
                            t.layers.push(LayerStatus {
                                layer_id: layer_id.clone(),
                                state: state.clone(),
                                current,
                                total,
                                progress: layer_progress,
                            });
                            t.total_layers = t.layers.len();
                        }

                        // Count completed layers
                        t.completed_layers = t.layers.iter()
                            .filter(|l| matches!(l.state, LayerState::Complete))
                            .count();

                        // Calculate overall progress
                        t.progress = calculate_overall_progress(t);
                        t.current_layer = layer_id;
                    }

                    t.message = last_status.clone();
                });

                // Check for errors
                if status.error_detail.is_some() {
                    update_task(&task_id, |t| {
                        t.status = TaskStatus::Failed;
                        t.message = format!("Pull failed: {:?}", status.error_detail);
                    });
                    return;
                }
            }
            Err(e) => {
                update_task(&task_id, |t| {
                    t.status = TaskStatus::Failed;
                    t.message = format!("Pull error: {}", e);
                });
                return;
            }
        }
    }

    // Complete
    update_task(&task_id, |t| {
        t.status = TaskStatus::Completed;
        t.message = format!("Image pulled successfully: {}", last_status);
        t.progress = 100;
        t.completed_layers = t.total_layers;
        t.current_layer = String::new();
    });
}

/// Calculate overall progress based on all layers
/// Weighted formula: completed_layers * 100% + downloading_layer_progress
fn calculate_overall_progress(task: &PullTask) -> u8 {
    if task.total_layers == 0 {
        return 5; // Initial phase
    }

    // Weight distribution:
    // - Complete layers: 100% each
    // - Downloading layers: progress varies (0-90%)
    // - Extracting layers: 95% each
    // - Waiting layers: 0%
    
    let total_weight: f64 = task.total_layers as f64 * 100.0;
    let mut accumulated_weight: f64 = 0.0;

    for layer in &task.layers {
        accumulated_weight += match layer.state {
            LayerState::Complete => 100.0,
            LayerState::Extracting => 95.0,
            LayerState::Downloading => layer.progress as f64 * 0.9, // Max 90% during download
            LayerState::Waiting => 0.0,
        };
    }

    // Calculate percentage (5-100 range)
    let progress = (accumulated_weight / total_weight * 95.0) as u8 + 5;
    progress.min(100)
}

// ============================================================================
// Create Container Related Structures
// ============================================================================

/// Port mapping configuration
#[derive(Deserialize, Debug)]
pub struct PortMapping {
    /// Host port (e.g., "8080")
    pub host_port: String,
    /// Container port (e.g., "80/tcp")
    pub container_port: String,
}

/// Volume mount configuration
#[derive(Deserialize, Debug)]
pub struct VolumeMount {
    /// Host path (e.g., "/data/myapp")
    pub host_path: String,
    /// Container path (e.g., "/app/data")
    pub container_path: String,
    /// Read-only mount
    #[serde(default)]
    pub read_only: bool,
}

/// Create Container Request
/// 
/// # Fields
/// - `image`: Docker image name (e.g., "nginx:latest", "ubuntu:22.04")
/// - `name`: Optional container name
/// - `cmd`: Optional command to run (e.g., ["echo", "hello"])
/// - `env`: Optional environment variables (e.g., {"KEY": "value"})
/// - `ports`: Optional port mappings (host_port -> container_port)
/// - `volumes`: Optional volume mounts
/// - `restart_policy`: Optional restart policy ("no", "always", "unless-stopped", "on-failure")
/// - `auto_start`: Whether to start the container immediately (default: true)
#[derive(Deserialize, Debug)]
pub struct CreateContainerRequest {
    /// Docker image name (required)
    pub image: String,
    /// Container name (optional, auto-generated if not provided)
    #[serde(default)]
    pub name: Option<String>,
    /// Command to run (optional)
    #[serde(default)]
    pub cmd: Option<Vec<String>>,
    /// Environment variables (optional)
    #[serde(default)]
    pub env: Option<HashMap<String, String>>,
    /// Port mappings (optional)
    #[serde(default)]
    pub ports: Option<Vec<PortMapping>>,
    /// Volume mounts (optional)
    #[serde(default)]
    pub volumes: Option<Vec<VolumeMount>>,
    /// Restart policy: "no", "always", "unless-stopped", "on-failure" (default: "no")
    #[serde(default = "default_restart_policy")]
    pub restart_policy: String,
    /// Auto start container after creation (default: true)
    #[serde(default = "default_auto_start")]
    pub auto_start: bool,
}

fn default_restart_policy() -> String {
    "no".to_string()
}

fn default_auto_start() -> bool {
    true
}

/// Create Container Response
#[derive(Serialize)]
pub struct CreateContainerResponse {
    pub success: bool,
    pub message: String,
    pub container_id: Option<String>,
    pub warnings: Option<Vec<String>>,
}

/// Validate restart policy
fn validate_restart_policy(policy: &str) -> Result<(), String> {
    match policy {
        "no" | "always" | "unless-stopped" | "on-failure" => Ok(()),
        _ => Err(format!("Invalid restart policy: {}. Valid values: no, always, unless-stopped, on-failure", policy)),
    }
}

/// Create Docker Container (Admin Only)
///
/// # Endpoint
/// POST /docker/create_container
///
/// # Request Body Example
/// ```json
/// {
///     "image": "nginx:latest",
///     "name": "my-nginx",
///     "cmd": null,
///     "env": {"NGINX_HOST": "example.com"},
///     "ports": [{"host_port": "8080", "container_port": "80/tcp"}],
///     "volumes": [{"host_path": "/data/nginx", "container_path": "/usr/share/nginx/html", "read_only": false}],
///     "restart_policy": "always",
///     "auto_start": true
/// }
/// ```
///
/// # Response
/// ```json
/// {
///     "success": true,
///     "message": "Container created successfully",
///     "container_id": "abc123...",
///     "warnings": []
/// }
/// ```
#[post("/docker/create_container")]
pub async fn create_container(
    req: HttpRequest,
    pool: web::Data<DbPool>,
    body: web::Json<CreateContainerRequest>,
) -> impl Responder {
    // 1. Verify admin access
    let _admin_username = match verify_admin_access(&req, &pool).await {
        Ok(username) => username,
        Err(response) => return response,
    };

    // 2. Validate request
    let req_body = body.into_inner();
    
    if req_body.image.trim().is_empty() {
        return HttpResponse::BadRequest().json(CreateContainerResponse {
            success: false,
            message: "Image name is required".to_string(),
            container_id: None,
            warnings: None,
        });
    }

    if let Err(e) = validate_restart_policy(&req_body.restart_policy) {
        return HttpResponse::BadRequest().json(CreateContainerResponse {
            success: false,
            message: e,
            container_id: None,
            warnings: None,
        });
    }

    // 3. Validate volume host paths against forbidden directories
    if let Some(volumes) = &req_body.volumes {
        for vol in volumes {
            if let Some(forbidden) = is_forbidden_path(&vol.host_path) {
                return HttpResponse::BadRequest().json(CreateContainerResponse {
                    success: false,
                    message: format!(
                        "Host path '{}' in volume is not allowed: it conflicts with forbidden directory '{}'",
                        vol.host_path, forbidden
                    ),
                    container_id: None,
                    warnings: None,
                });
            }
        }
    }

    // 4. Connect to Docker
    let docker = match Docker::connect_with_local_defaults() {
        Ok(d) => d,
        Err(e) => {
            return HttpResponse::InternalServerError().json(CreateContainerResponse {
                success: false,
                message: format!("Failed to connect to Docker: {}", e),
                container_id: None,
                warnings: None,
            });
        }
    };

    // 5. Check if image exists locally
    let image_name = req_body.image.trim().to_string();
    let image_exists = match docker.list_images(Some(ListImagesOptions {
        all: true,
        ..Default::default()
    })).await {
        Ok(images) => images.iter().any(|img| {
            img.repo_tags.iter().any(|tag| {
                tag == &image_name || tag.starts_with(&format!("{}:", image_name))
            })
        }),
        Err(_) => false,
    };

    if !image_exists {
        return HttpResponse::BadRequest().json(CreateContainerResponse {
            success: false,
            message: format!("Image '{}' not found locally. Please pull the image first using /docker/pull_image/start", image_name),
            container_id: None,
            warnings: None,
        });
    }

    // 6. Build container configuration
    let cmd = req_body.cmd.clone();
    
    // Set environment variables
    let env: Option<Vec<String>> = req_body.env.as_ref().map(|env_map| {
        env_map
            .iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect()
    });

    // Build port bindings and exposed ports
    let mut port_bindings: HashMap<String, Option<Vec<PortBinding>>> = HashMap::new();
    let mut exposed_ports: Option<Vec<String>> = None;

    if let Some(ports) = &req_body.ports {
        let mut exposed = Vec::new();
        for port_mapping in ports {
            let container_port = &port_mapping.container_port;
            let host_port = &port_mapping.host_port;

            port_bindings.insert(
                container_port.clone(),
                Some(vec![PortBinding {
                    host_ip: Some("0.0.0.0".to_string()),
                    host_port: Some(host_port.clone()),
                }]),
            );
            exposed.push(container_port.clone());
        }
        if !exposed.is_empty() {
            exposed_ports = Some(exposed);
        }
    }

    // Build mounts
    let mounts: Option<Vec<Mount>> = req_body.volumes.as_ref().map(|volumes| {
        volumes
            .iter()
            .map(|vol| Mount {
                target: Some(vol.container_path.clone()),
                source: Some(vol.host_path.clone()),
                typ: Some(MountTypeEnum::BIND),
                read_only: Some(vol.read_only),
                ..Default::default()
            })
            .collect()
    });

    // Build host config
    let mut host_config = HostConfig {
        port_bindings: if port_bindings.is_empty() {
            None
        } else {
            Some(port_bindings)
        },
        mounts,
        ..Default::default()
    };

    // Set restart policy
    match req_body.restart_policy.as_str() {
        "always" => {
            host_config.restart_policy = Some(bollard::models::RestartPolicy {
                name: Some(bollard::models::RestartPolicyNameEnum::ALWAYS),
                maximum_retry_count: None,
            });
        }
        "unless-stopped" => {
            host_config.restart_policy = Some(bollard::models::RestartPolicy {
                name: Some(bollard::models::RestartPolicyNameEnum::UNLESS_STOPPED),
                maximum_retry_count: None,
            });
        }
        "on-failure" => {
            host_config.restart_policy = Some(bollard::models::RestartPolicy {
                name: Some(bollard::models::RestartPolicyNameEnum::ON_FAILURE),
                maximum_retry_count: Some(5),
            });
        }
        _ => {
            // "no" - no restart policy
            host_config.restart_policy = Some(bollard::models::RestartPolicy {
                name: Some(bollard::models::RestartPolicyNameEnum::EMPTY),
                maximum_retry_count: None,
            });
        }
    }

    // Build container create body
    let config = ContainerCreateBody {
        image: Some(image_name.clone()),
        cmd,
        env,
        exposed_ports,
        host_config: Some(host_config),
        ..Default::default()
    };

    // 7. Create container
    let options = CreateContainerOptions {
        name: req_body.name.clone(),
        ..Default::default()
    };

    match docker.create_container(Some(options), config).await {
        Ok(ContainerCreateResponse { id, warnings }) => {
            let short_id = format_short_id(&id);
            
            // 8. Auto start container if requested
            if req_body.auto_start {
                match docker.start_container(&id, None).await {
                    Ok(_) => {
                        HttpResponse::Ok().json(CreateContainerResponse {
                            success: true,
                            message: format!("Container '{}' created and started successfully", short_id),
                            container_id: Some(id),
                            warnings: if warnings.is_empty() { None } else { Some(warnings) },
                        })
                    }
                    Err(e) => {
                        // Container created but failed to start
                        HttpResponse::Ok().json(CreateContainerResponse {
                            success: true,
                            message: format!(
                                "Container '{}' created successfully but failed to start: {}. Start manually with: docker start {}",
                                short_id, e, short_id
                            ),
                            container_id: Some(id),
                            warnings: if warnings.is_empty() { 
                                Some(vec![format!("Failed to start: {}", e)]) 
                            } else { 
                                Some([warnings, vec![format!("Failed to start: {}", e)]].concat()) 
                            },
                        })
                    }
                }
            } else {
                HttpResponse::Ok().json(CreateContainerResponse {
                    success: true,
                    message: format!("Container '{}' created successfully (not started)", short_id),
                    container_id: Some(id),
                    warnings: if warnings.is_empty() { None } else { Some(warnings) },
                })
            }
        }
        Err(e) => HttpResponse::InternalServerError().json(CreateContainerResponse {
            success: false,
            message: format!("Failed to create container: {}", e),
            container_id: None,
            warnings: None,
        }),
    }
}

// ============================================================================
// Registry Mirror Management
// ============================================================================

const REGISTRY_CONF_PATH: &str = "/etc/containers/registries.conf";
const REGISTRY_CONF_BACKUP_PATH: &str = "/etc/containers/registries.conf.bak";

/// Registry mirror configuration entry
#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct RegistryMirrorEntry {
    /// Registry prefix to match (e.g., "docker.io")
    pub prefix: String,
    /// Mirror location (e.g., "docker.nju.edu.cn")
    pub location: String,
    /// Whether to allow insecure (http) connection
    #[serde(default)]
    pub insecure: bool,
}

/// Add/Update registry mirror request
#[derive(Deserialize, Debug)]
pub struct SetRegistryRequest {
    /// Registry prefix (e.g., "docker.io", "docker.io/library")
    pub prefix: String,
    /// Mirror location (e.g., "docker.nju.edu.cn", "hub-mirror.c.163.com")
    pub location: String,
    /// Allow insecure http connection
    #[serde(default)]
    pub insecure: bool,
}

/// Registry mirror response
#[derive(Serialize)]
pub struct RegistryResponse {
    pub success: bool,
    pub message: String,
    pub mirrors: Option<Vec<RegistryMirrorEntry>>,
}

/// Read existing registry configuration
fn read_registry_config() -> Result<String, String> {
    if Path::new(REGISTRY_CONF_PATH).exists() {
        fs::read_to_string(REGISTRY_CONF_PATH)
            .map_err(|e| format!("Failed to read registry config: {}", e))
    } else {
        Ok(String::new())
    }
}

/// Parse registry config and extract mirror entries
fn parse_registry_mirrors(content: &str) -> Vec<RegistryMirrorEntry> {
    let mut mirrors = Vec::new();
    let mut current_prefix = String::new();
    let mut current_location = String::new();
    let mut current_insecure = false;
    let mut in_registry_block = false;

    for line in content.lines() {
        let trimmed = line.trim();
        
        // Check for [[registry]] block start
        if trimmed == "[[registry]]" {
            // Save previous entry if exists
            if !current_prefix.is_empty() && !current_location.is_empty() {
                mirrors.push(RegistryMirrorEntry {
                    prefix: current_prefix.clone(),
                    location: current_location.clone(),
                    insecure: current_insecure,
                });
            }
            // Reset for new block
            current_prefix.clear();
            current_location.clear();
            current_insecure = false;
            in_registry_block = true;
        }
        // Check for [[registry.mirror]] block
        else if trimmed == "[[registry.mirror]]" {
            // Save primary registry entry if exists
            if !current_prefix.is_empty() && !current_location.is_empty() {
                mirrors.push(RegistryMirrorEntry {
                    prefix: current_prefix.clone(),
                    location: current_location.clone(),
                    insecure: current_insecure,
                });
            }
            // Clear location for mirror (keep prefix)
            current_location.clear();
            current_insecure = false;
        }
        // Parse prefix
        else if trimmed.starts_with("prefix") && in_registry_block {
            if let Some(value) = trimmed.split('=').nth(1) {
                current_prefix = value.trim().trim_matches('"').to_string();
            }
        }
        // Parse location
        else if trimmed.starts_with("location") && in_registry_block {
            if let Some(value) = trimmed.split('=').nth(1) {
                current_location = value.trim().trim_matches('"').to_string();
            }
        }
        // Parse insecure
        else if trimmed.starts_with("insecure") && in_registry_block {
            if let Some(value) = trimmed.split('=').nth(1) {
                current_insecure = value.trim().parse::<bool>().unwrap_or(false);
            }
        }
    }

    // Save last entry
    if !current_prefix.is_empty() && !current_location.is_empty() {
        mirrors.push(RegistryMirrorEntry {
            prefix: current_prefix,
            location: current_location,
            insecure: current_insecure,
        });
    }

    mirrors
}

/// Generate registry configuration content
fn generate_registry_config(mirrors: &[RegistryMirrorEntry]) -> String {
    let mut content = String::new();
    
    // Add header comment
    content.push_str("# Podman Registry Configuration\n");
    content.push_str("# Generated by zuti\n\n");
    
    // Collect all unique prefixes
    let mut prefix_map: HashMap<String, Vec<&RegistryMirrorEntry>> = HashMap::new();
    for mirror in mirrors {
        prefix_map.entry(mirror.prefix.clone())
            .or_default()
            .push(mirror);
    }

    // Add unqualified-search-registries
    let search_registries: Vec<String> = prefix_map.keys()
        .filter(|k| k == &"docker.io" || k == &"quay.io" || k == &"ghcr.io" || k == &"gcr.io")
        .cloned()
        .collect();
    
    if !search_registries.is_empty() {
        content.push_str("unqualified-search-registries = [");
        let reg_list: Vec<String> = search_registries.iter()
            .map(|r| format!("\"{}\"", r))
            .collect();
        content.push_str(&reg_list.join(", "));
        content.push_str("]\n\n");
    }

    // Generate registry blocks
    for (prefix, entries) in prefix_map {
        if let Some(first) = entries.first() {
            content.push_str("[[registry]]\n");
            content.push_str(&format!("prefix = \"{}\"\n", prefix));
            content.push_str(&format!("location = \"{}\"\n", first.location));
            if first.insecure {
                content.push_str("insecure = true\n");
            }
            content.push('\n');

            // Add mirrors
            for entry in entries.iter().skip(1) {
                content.push_str("[[registry.mirror]]\n");
                content.push_str(&format!("location = \"{}\"\n", entry.location));
                if entry.insecure {
                    content.push_str("insecure = true\n");
                }
                content.push('\n');
            }
        }
    }

    content
}

/// Add or update registry mirror
/// 
/// # Endpoint
/// POST /docker/setting/registry
///
/// # Request Body
/// ```json
/// {
///     "prefix": "docker.io",
///     "location": "docker.nju.edu.cn",
///     "insecure": false
/// }
/// ```
#[post("/docker/setting/registry")]
pub async fn set_registry_mirror(
    req: HttpRequest,
    pool: web::Data<DbPool>,
    body: web::Json<SetRegistryRequest>,
) -> impl Responder {
    // 1. Verify admin access
    let _admin_username = match verify_admin_access(&req, &pool).await {
        Ok(username) => username,
        Err(response) => return response,
    };

    // 2. Validate request
    let req_body = body.into_inner();
    let prefix = req_body.prefix.trim();
    let location = req_body.location.trim();

    if prefix.is_empty() {
        return HttpResponse::BadRequest().json(RegistryResponse {
            success: false,
            message: "Prefix is required".to_string(),
            mirrors: None,
        });
    }

    if location.is_empty() {
        return HttpResponse::BadRequest().json(RegistryResponse {
            success: false,
            message: "Location is required".to_string(),
            mirrors: None,
        });
    }

    // 3. Ensure /etc/containers directory exists
    let conf_dir = Path::new("/etc/containers");
    if !conf_dir.exists() {
        match fs::create_dir_all(conf_dir) {
            Ok(_) => {},
            Err(e) => {
                return HttpResponse::InternalServerError().json(RegistryResponse {
                    success: false,
                    message: format!("Failed to create /etc/containers directory: {}", e),
                    mirrors: None,
                });
            }
        }
    }

    // 4. Backup existing config if exists
    if Path::new(REGISTRY_CONF_PATH).exists() {
        if let Err(e) = fs::copy(REGISTRY_CONF_PATH, REGISTRY_CONF_BACKUP_PATH) {
            return HttpResponse::InternalServerError().json(RegistryResponse {
                success: false,
                message: format!("Failed to backup registry config: {}", e),
                mirrors: None,
            });
        }
    }

    // 5. Read existing mirrors
    let existing_content = match read_registry_config() {
        Ok(content) => content,
        Err(e) => {
            return HttpResponse::InternalServerError().json(RegistryResponse {
                success: false,
                message: e,
                mirrors: None,
            });
        }
    };

    let mut mirrors = parse_registry_mirrors(&existing_content);

    // 6. Check if this exact prefix+location already exists
    let existing_index = mirrors.iter().position(|m| {
        m.prefix == prefix && m.location == location
    });

    if let Some(index) = existing_index {
        // Update existing entry
        mirrors[index].insecure = req_body.insecure;
    } else {
        // Add new entry
        mirrors.push(RegistryMirrorEntry {
            prefix: prefix.to_string(),
            location: location.to_string(),
            insecure: req_body.insecure,
        });
    }

    // 7. Generate new config content
    let new_content = generate_registry_config(&mirrors);

    // 8. Write to file
    let mut file = match fs::File::create(REGISTRY_CONF_PATH) {
        Ok(f) => f,
        Err(e) => {
            return HttpResponse::InternalServerError().json(RegistryResponse {
                success: false,
                message: format!("Failed to create registry config file: {}", e),
                mirrors: None,
            });
        }
    };

    if let Err(e) = file.write_all(new_content.as_bytes()) {
        return HttpResponse::InternalServerError().json(RegistryResponse {
            success: false,
            message: format!("Failed to write registry config: {}", e),
            mirrors: None,
        });
    }

    HttpResponse::Ok().json(RegistryResponse {
        success: true,
        message: format!("Registry mirror '{}' -> '{}' configured successfully", prefix, location),
        mirrors: Some(mirrors),
    })
}

/// Get all registry mirrors
///
/// # Endpoint
/// GET /docker/setting/registry
#[get("/docker/setting/registry")]
pub async fn get_registry_mirrors(
    req: HttpRequest,
    pool: web::Data<DbPool>,
) -> impl Responder {
    // 1. Verify admin access
    let _admin_username = match verify_admin_access(&req, &pool).await {
        Ok(username) => username,
        Err(response) => return response,
    };

    // 2. Read config
    let content = match read_registry_config() {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError().json(RegistryResponse {
                success: false,
                message: e,
                mirrors: None,
            });
        }
    };

    if content.is_empty() {
        return HttpResponse::Ok().json(RegistryResponse {
            success: true,
            message: "No registry mirrors configured".to_string(),
            mirrors: Some(vec![]),
        });
    }

    let mirrors = parse_registry_mirrors(&content);

    HttpResponse::Ok().json(RegistryResponse {
        success: true,
        message: format!("Found {} registry mirror(s)", mirrors.len()),
        mirrors: Some(mirrors),
    })
}

/// Delete registry mirror
///
/// # Endpoint
/// DELETE /docker/setting/registry
///
/// # Request Body
/// ```json
/// {
///     "prefix": "docker.io",
///     "location": "docker.nju.edu.cn"
/// }
/// ```
#[delete("/docker/setting/registry")]
pub async fn delete_registry_mirror(
    req: HttpRequest,
    pool: web::Data<DbPool>,
    body: web::Json<SetRegistryRequest>,
) -> impl Responder {
    // 1. Verify admin access
    let _admin_username = match verify_admin_access(&req, &pool).await {
        Ok(username) => username,
        Err(response) => return response,
    };

    // 2. Validate request
    let req_body = body.into_inner();
    let prefix = req_body.prefix.trim();
    let location = req_body.location.trim();

    if prefix.is_empty() || location.is_empty() {
        return HttpResponse::BadRequest().json(RegistryResponse {
            success: false,
            message: "Both prefix and location are required".to_string(),
            mirrors: None,
        });
    }

    // 3. Read existing mirrors
    let existing_content = match read_registry_config() {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError().json(RegistryResponse {
                success: false,
                message: e,
                mirrors: None,
            });
        }
    };

    let mut mirrors = parse_registry_mirrors(&existing_content);
    let original_len = mirrors.len();

    // 4. Remove matching entry
    mirrors.retain(|m| !(m.prefix == prefix && m.location == location));

    if mirrors.len() == original_len {
        return HttpResponse::NotFound().json(RegistryResponse {
            success: false,
            message: format!("Registry mirror '{} -> {}' not found", prefix, location),
            mirrors: None,
        });
    }

    // 5. Backup and write new config
    if let Err(e) = fs::copy(REGISTRY_CONF_PATH, REGISTRY_CONF_BACKUP_PATH) {
        return HttpResponse::InternalServerError().json(RegistryResponse {
            success: false,
            message: format!("Failed to backup registry config: {}", e),
            mirrors: None,
        });
    }

    let new_content = generate_registry_config(&mirrors);
    let mut file = match fs::File::create(REGISTRY_CONF_PATH) {
        Ok(f) => f,
        Err(e) => {
            return HttpResponse::InternalServerError().json(RegistryResponse {
                success: false,
                message: format!("Failed to create registry config file: {}", e),
                mirrors: None,
            });
        }
    };

    if let Err(e) = file.write_all(new_content.as_bytes()) {
        return HttpResponse::InternalServerError().json(RegistryResponse {
            success: false,
            message: format!("Failed to write registry config: {}", e),
            mirrors: None,
        });
    }

    HttpResponse::Ok().json(RegistryResponse {
        success: true,
        message: format!("Registry mirror '{} -> {}' deleted successfully", prefix, location),
        mirrors: Some(mirrors),
    })
}

// ============================================================================
// Registry Mirror Management (for [[registry.mirror]] blocks)
// ============================================================================

/// Add mirror request - adds a mirror to existing registry
#[derive(Deserialize, Debug)]
pub struct AddMirrorRequest {
    /// Mirror location (e.g., "hub-mirror.c.163.com")
    pub location: String,
    /// Allow insecure http connection
    #[serde(default)]
    pub insecure: bool,
}

/// Mirror entry in group
#[derive(Serialize, Debug, Clone)]
pub struct MirrorEntry {
    pub location: String,
    pub insecure: bool,
}

/// Mirrors grouped by prefix
#[derive(Serialize, Debug)]
pub struct MirrorGroup {
    pub prefix: String,
    pub primary: MirrorEntry,
    pub mirrors: Vec<MirrorEntry>,
}

/// Mirror list response
#[derive(Serialize)]
pub struct MirrorListResponse {
    pub success: bool,
    pub message: String,
    pub groups: Option<Vec<MirrorGroup>>,
}

/// Parse registry config into groups (primary + mirrors)
fn parse_mirror_groups(content: &str) -> Vec<MirrorGroup> {
    let mut groups: Vec<MirrorGroup> = Vec::new();
    let mut current_prefix = String::new();
    let mut current_primary: Option<MirrorEntry> = None;
    let mut current_mirrors: Vec<MirrorEntry> = Vec::new();
    let mut in_registry_block = false;
    let mut is_mirror_block = false;

    for line in content.lines() {
        let trimmed = line.trim();
        
        if trimmed == "[[registry]]" {
            // Save previous group if exists
            if !current_prefix.is_empty() && current_primary.is_some() {
                groups.push(MirrorGroup {
                    prefix: current_prefix.clone(),
                    primary: current_primary.unwrap(),
                    mirrors: current_mirrors.clone(),
                });
            }
            // Reset for new block
            current_prefix.clear();
            current_primary = None;
            current_mirrors.clear();
            in_registry_block = true;
            is_mirror_block = false;
        }
        else if trimmed == "[[registry.mirror]]" {
            is_mirror_block = true;
        }
        else if trimmed.starts_with("prefix") && in_registry_block {
            if let Some(value) = trimmed.split('=').nth(1) {
                current_prefix = value.trim().trim_matches('"').to_string();
            }
        }
        else if trimmed.starts_with("location") && in_registry_block {
            if let Some(value) = trimmed.split('=').nth(1) {
                let location = value.trim().trim_matches('"').to_string();
                let insecure = false; // Simplified parsing
                
                let entry = MirrorEntry { location, insecure };
                
                if is_mirror_block {
                    current_mirrors.push(entry);
                } else {
                    current_primary = Some(entry);
                }
            }
        }
    }

    // Save last group
    if !current_prefix.is_empty() && current_primary.is_some() {
        groups.push(MirrorGroup {
            prefix: current_prefix,
            primary: current_primary.unwrap(),
            mirrors: current_mirrors,
        });
    }

    groups
}

/// Generate config from mirror groups
fn generate_config_from_groups(groups: &[MirrorGroup]) -> String {
    let mut content = String::new();
    
    content.push_str("# Podman Registry Configuration\n");
    content.push_str("# Generated by zuti\n\n");
    
    // Collect unique prefixes for search registries
    let prefixes: Vec<String> = groups.iter().map(|g| g.prefix.clone()).collect();
    if !prefixes.is_empty() {
        content.push_str("unqualified-search-registries = [");
        let reg_list: Vec<String> = prefixes.iter()
            .map(|r| format!("\"{}\"", r))
            .collect();
        content.push_str(&reg_list.join(", "));
        content.push_str("]\n\n");
    }

    for group in groups {
        // Primary registry
        content.push_str("[[registry]]\n");
        content.push_str(&format!("prefix = \"{}\"\n", group.prefix));
        content.push_str(&format!("location = \"{}\"\n", group.primary.location));
        if group.primary.insecure {
            content.push_str("insecure = true\n");
        }
        content.push('\n');

        // Mirrors
        for mirror in &group.mirrors {
            content.push_str("[[registry.mirror]]\n");
            content.push_str(&format!("location = \"{}\"\n", mirror.location));
            if mirror.insecure {
                content.push_str("insecure = true\n");
            }
            content.push('\n');
        }
    }

    content
}

/// Add a mirror to existing registry (uses first available registry or docker.io)
///
/// # Endpoint
/// POST /docker/setting/mirror
#[post("/docker/setting/mirror")]
pub async fn add_registry_mirror(
    req: HttpRequest,
    pool: web::Data<DbPool>,
    body: web::Json<AddMirrorRequest>,
) -> impl Responder {
    // 1. Verify admin access
    let _admin_username = match verify_admin_access(&req, &pool).await {
        Ok(username) => username,
        Err(response) => return response,
    };

    // 2. Validate request
    let req_body = body.into_inner();
    let location = req_body.location.trim();

    if location.is_empty() {
        return HttpResponse::BadRequest().json(MirrorListResponse {
            success: false,
            message: "Mirror location is required".to_string(),
            groups: None,
        });
    }

    // 3. Read existing config
    let content = read_registry_config().unwrap_or_default();
    let mut groups = parse_mirror_groups(&content);

    // 4. Find registry group to add mirror to
    // Priority: docker.io > first available
    let group_index = groups.iter().position(|g| g.prefix == "docker.io")
        .or_else(|| if groups.is_empty() { None } else { Some(0) });
    
    let prefix = match group_index {
        Some(index) => {
            let prefix = groups[index].prefix.clone();
            
            // Check if mirror already exists in any registry
            for group in &groups {
                if group.mirrors.iter().any(|m| m.location == location) {
                    return HttpResponse::BadRequest().json(MirrorListResponse {
                        success: false,
                        message: format!("Mirror '{}' already exists", location),
                        groups: Some(groups),
                    });
                }
            }
            
            // Add mirror
            groups[index].mirrors.push(MirrorEntry {
                location: location.to_string(),
                insecure: req_body.insecure,
            });
            prefix
        }
        None => {
            return HttpResponse::BadRequest().json(MirrorListResponse {
                success: false,
                message: "No primary registry found. Please create it first using /docker/setting/registry".to_string(),
                groups: Some(groups),
            });
        }
    };

    // 5. Backup and write config
    if let Err(e) = fs::copy(REGISTRY_CONF_PATH, REGISTRY_CONF_BACKUP_PATH) {
        return HttpResponse::InternalServerError().json(MirrorListResponse {
            success: false,
            message: format!("Failed to backup config: {}", e),
            groups: None,
        });
    }

    let new_content = generate_config_from_groups(&groups);
    if let Err(e) = fs::write(REGISTRY_CONF_PATH, new_content) {
        return HttpResponse::InternalServerError().json(MirrorListResponse {
            success: false,
            message: format!("Failed to write config: {}", e),
            groups: None,
        });
    }

    HttpResponse::Ok().json(MirrorListResponse {
        success: true,
        message: format!("Mirror '{}' added to prefix '{}'", location, prefix),
        groups: Some(groups),
    })
}

/// List all registry mirrors grouped by prefix
///
/// # Endpoint
/// GET /docker/setting/mirror
#[get("/docker/setting/mirror")]
pub async fn list_registry_mirrors(
    req: HttpRequest,
    pool: web::Data<DbPool>,
) -> impl Responder {
    // 1. Verify admin access
    let _admin_username = match verify_admin_access(&req, &pool).await {
        Ok(username) => username,
        Err(response) => return response,
    };

    // 2. Read config
    let content = match read_registry_config() {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError().json(MirrorListResponse {
                success: false,
                message: e,
                groups: None,
            });
        }
    };

    if content.is_empty() {
        return HttpResponse::Ok().json(MirrorListResponse {
            success: true,
            message: "No registry configuration found".to_string(),
            groups: Some(vec![]),
        });
    }

    let groups = parse_mirror_groups(&content);

    HttpResponse::Ok().json(MirrorListResponse {
        success: true,
        message: format!("Found {} registry group(s)", groups.len()),
        groups: Some(groups),
    })
}

/// Delete a mirror from registry by location (searches all registries)
///
/// # Endpoint
/// DELETE /docker/setting/mirror
#[delete("/docker/setting/mirror")]
pub async fn delete_registry_mirror_entry(
    req: HttpRequest,
    pool: web::Data<DbPool>,
    body: web::Json<AddMirrorRequest>,
) -> impl Responder {
    // 1. Verify admin access
    let _admin_username = match verify_admin_access(&req, &pool).await {
        Ok(username) => username,
        Err(response) => return response,
    };

    // 2. Validate request
    let req_body = body.into_inner();
    let location = req_body.location.trim();

    if location.is_empty() {
        return HttpResponse::BadRequest().json(MirrorListResponse {
            success: false,
            message: "Mirror location is required".to_string(),
            groups: None,
        });
    }

    // 3. Read existing config
    let content = match read_registry_config() {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError().json(MirrorListResponse {
                success: false,
                message: e,
                groups: None,
            });
        }
    };

    let mut groups = parse_mirror_groups(&content);
    let mut found = false;
    let mut affected_prefix = String::new();

    // 4. Search all groups and remove mirror by location
    for group in &mut groups {
        let original_len = group.mirrors.len();
        group.mirrors.retain(|m| m.location != location);
        if group.mirrors.len() < original_len {
            found = true;
            affected_prefix = group.prefix.clone();
        }
    }

    if !found {
        return HttpResponse::NotFound().json(MirrorListResponse {
            success: false,
            message: format!("Mirror '{}' not found", location),
            groups: Some(groups),
        });
    }

    // 5. Backup and write config
    if let Err(e) = fs::copy(REGISTRY_CONF_PATH, REGISTRY_CONF_BACKUP_PATH) {
        return HttpResponse::InternalServerError().json(MirrorListResponse {
            success: false,
            message: format!("Failed to backup config: {}", e),
            groups: None,
        });
    }

    let new_content = generate_config_from_groups(&groups);
    if let Err(e) = fs::write(REGISTRY_CONF_PATH, new_content) {
        return HttpResponse::InternalServerError().json(MirrorListResponse {
            success: false,
            message: format!("Failed to write config: {}", e),
            groups: None,
        });
    }

    HttpResponse::Ok().json(MirrorListResponse {
        success: true,
        message: format!("Mirror '{}' deleted from prefix '{}'", location, affected_prefix),
        groups: Some(groups),
    })
}

// ============================================================================
// Volume Setting Management (Stored in ZUTI_SETTING_FILE)
// ============================================================================

/// Volume configuration entry
#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct VolumeSetting {
    /// Volume name/identifier
    pub name: String,
    /// Host path for the volume
    pub host_path: String,
    /// Volume description/notes
    #[serde(default)]
    pub description: Option<String>,
}

/// Volume configuration wrapper for the setting file
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct VolumeConfig {
    #[serde(default)]
    pub volumes: Vec<VolumeSetting>,
}

/// Set/Add volume request
#[derive(Deserialize, Debug)]
pub struct SetVolumeRequest {
    pub name: String,
    pub host_path: String,
    pub description: Option<String>,
}

/// Delete volume request
#[derive(Deserialize, Debug)]
pub struct DeleteVolumeRequest {
    pub name: String,
}

/// Volume response
#[derive(Serialize)]
pub struct VolumeResponse {
    pub success: bool,
    pub message: String,
    pub volumes: Option<Vec<VolumeSetting>>,
}

/// Read volume configuration from ZUTI_SETTING_FILE
fn read_volume_config() -> Result<VolumeConfig, String> {
    let path = Path::new(ZUTI_SETTING_FILE);
    if !path.exists() {
        return Ok(VolumeConfig::default());
    }

    let content = fs::read_to_string(path)
        .map_err(|e| format!("Failed to read volume config: {}", e))?;

    if content.trim().is_empty() {
        return Ok(VolumeConfig::default());
    }

    // Parse as JSON, if fails, try to read as TOML
    match serde_json::from_str::<VolumeConfig>(&content) {
        Ok(config) => Ok(config),
        Err(_) => {
            // Try TOML format
            toml::from_str::<VolumeConfig>(&content)
                .map_err(|e| format!("Failed to parse volume config: {}", e))
        }
    }
}

/// Check if host_path is in forbidden directory list
fn is_forbidden_path(host_path: &str) -> Option<&'static str> {
    let normalized_path = if host_path.ends_with('/') {
        &host_path[..host_path.len()-1]
    } else {
        host_path
    };
    
    for &forbidden in FORBID_DIRECTORY {
        // Check if path is exactly the forbidden directory or is a subdirectory
        if normalized_path == forbidden || normalized_path.starts_with(&format!("{}/", forbidden)) {
            return Some(forbidden);
        }
    }
    None
}

/// Write volume configuration to ZUTI_SETTING_FILE
fn write_volume_config(config: &VolumeConfig) -> Result<(), String> {
    // Validate all host paths against forbidden directories
    for volume in &config.volumes {
        if let Some(forbidden) = is_forbidden_path(&volume.host_path) {
            return Err(format!(
                "Host path '{}' is not allowed: it conflicts with forbidden directory '{}'",
                volume.host_path, forbidden
            ));
        }
    }

    let path = Path::new(ZUTI_SETTING_FILE);

    // Create parent directory if not exists
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create config directory: {}", e))?;
        }
    }

    // Write as TOML format for better readability
    let content = toml::to_string_pretty(config)
        .map_err(|e| format!("Failed to serialize volume config: {}", e))?;

    fs::write(path, content)
        .map_err(|e| format!("Failed to write volume config: {}", e))
}

/// Set/Add volume configuration
///
/// # Endpoint
/// POST /docker/setting/volume
///
/// # Request Body
/// ```json
/// {
///     "name": "data-volume",
///     "host_path": "/data/myapp",
///     "description": "Application data volume"
/// }
/// ```
#[post("/docker/setting/volume")]
pub async fn set_volume(
    req: HttpRequest,
    pool: web::Data<DbPool>,
    body: web::Json<SetVolumeRequest>,
) -> impl Responder {
    // 1. Verify admin access
    let _admin_username = match verify_admin_access(&req, &pool).await {
        Ok(username) => username,
        Err(response) => return response,
    };

    // 2. Validate request
    let req_body = body.into_inner();
    let name = req_body.name.trim();
    let host_path = req_body.host_path.trim();

    if name.is_empty() {
        return HttpResponse::BadRequest().json(VolumeResponse {
            success: false,
            message: "Volume name is required".to_string(),
            volumes: None,
        });
    }

    if host_path.is_empty() {
        return HttpResponse::BadRequest().json(VolumeResponse {
            success: false,
            message: "Host path is required".to_string(),
            volumes: None,
        });
    }

    // 3. Read existing config
    let mut config = match read_volume_config() {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError().json(VolumeResponse {
                success: false,
                message: e,
                volumes: None,
            });
        }
    };

    // 4. Check if volume with same name exists, update or add
    let existing_index = config.volumes.iter().position(|v| v.name == name);

    let new_volume = VolumeSetting {
        name: name.to_string(),
        host_path: host_path.to_string(),
        description: req_body.description,
    };

    let message = if let Some(index) = existing_index {
        config.volumes[index] = new_volume;
        format!("Volume '{}' updated successfully", name)
    } else {
        config.volumes.push(new_volume);
        format!("Volume '{}' added successfully", name)
    };

    // 5. Write config
    if let Err(e) = write_volume_config(&config) {
        return HttpResponse::InternalServerError().json(VolumeResponse {
            success: false,
            message: e,
            volumes: None,
        });
    }

    HttpResponse::Ok().json(VolumeResponse {
        success: true,
        message,
        volumes: Some(config.volumes),
    })
}

/// Get all volume configurations
///
/// # Endpoint
/// GET /docker/setting/volume
#[get("/docker/setting/volume")]
pub async fn get_volumes(
    req: HttpRequest,
    pool: web::Data<DbPool>,
) -> impl Responder {
    // 1. Verify admin access
    let _admin_username = match verify_admin_access(&req, &pool).await {
        Ok(username) => username,
        Err(response) => return response,
    };

    // 2. Read config
    match read_volume_config() {
        Ok(config) => {
            let count = config.volumes.len();
            HttpResponse::Ok().json(VolumeResponse {
                success: true,
                message: format!("Found {} volume(s)", count),
                volumes: Some(config.volumes),
            })
        }
        Err(e) => HttpResponse::InternalServerError().json(VolumeResponse {
            success: false,
            message: e,
            volumes: None,
        }),
    }
}

/// Delete volume configuration
///
/// # Endpoint
/// DELETE /docker/setting/volume/{name}
///
/// # Path Parameters
/// - `name`: Volume name to delete
#[delete("/docker/setting/volume/{name}")]
pub async fn delete_volume(
    req: HttpRequest,
    pool: web::Data<DbPool>,
    path: web::Path<String>,
) -> impl Responder {
    // 1. Verify admin access
    let _admin_username = match verify_admin_access(&req, &pool).await {
        Ok(username) => username,
        Err(response) => return response,
    };

    // 2. Get volume name
    let name = path.into_inner().trim().to_string();

    if name.is_empty() {
        return HttpResponse::BadRequest().json(VolumeResponse {
            success: false,
            message: "Volume name is required".to_string(),
            volumes: None,
        });
    }

    // 3. Read existing config
    let mut config = match read_volume_config() {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError().json(VolumeResponse {
                success: false,
                message: e,
                volumes: None,
            });
        }
    };

    // 4. Find and remove volume
    let original_len = config.volumes.len();
    config.volumes.retain(|v| v.name != name);

    if config.volumes.len() == original_len {
        return HttpResponse::NotFound().json(VolumeResponse {
            success: false,
            message: format!("Volume '{}' not found", name),
            volumes: None,
        });
    }

    // 5. Write config
    if let Err(e) = write_volume_config(&config) {
        return HttpResponse::InternalServerError().json(VolumeResponse {
            success: false,
            message: e,
            volumes: None,
        });
    }

    HttpResponse::Ok().json(VolumeResponse {
        success: true,
        message: format!("Volume '{}' deleted successfully", name),
        volumes: Some(config.volumes),
    })
}
