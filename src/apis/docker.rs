use actix_web::{get, post, web, HttpRequest, HttpResponse, Responder};
use bollard::Docker;
use bollard::query_parameters::{CreateImageOptions, ListImagesOptions};
use futures_util::stream::StreamExt;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::utils::admin::verify_admin_access;
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
