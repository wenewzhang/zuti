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

/// Pull Task Information
#[derive(Serialize, Clone)]
pub struct PullTask {
    pub task_id: String,
    pub image_name: String,
    pub status: TaskStatus,
    pub progress: u8,           // 0-100
    pub current_layer: String,  // Current layer being processed
    pub message: String,        // Status description
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
    pub parent_id: String,
    pub repo_tags: Vec<String>,
    pub repo_digests: Vec<String>,
    pub created: i64,
    pub size: i64,
    pub shared_size: i64,
    pub labels: Option<std::collections::HashMap<String, String>>,
    pub containers: i64,
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
                    id: img.id,
                    parent_id: img.parent_id,
                    repo_tags: img.repo_tags,
                    repo_digests: img.repo_digests,
                    created: img.created,
                    size: img.size,
                    shared_size: img.shared_size,
                    labels: Some(img.labels),
                    containers: img.containers,
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
        t.progress = 10;
    });

    // Create pull options
    let options = Some(CreateImageOptions {
        from_image: Some(repo),
        tag: Some(tag),
        ..Default::default()
    });

    // Execute pull
    let mut stream = docker.create_image(options, None, None);
    let mut last_status = String::new();

    while let Some(result) = stream.next().await {
        match result {
            Ok(status) => {
                // Update status info
                if let Some(status_str) = &status.status {
                    last_status = status_str.clone();
                }

                // Calculate progress (layer-based estimate)
                let progress = calculate_progress(&status);

                update_task(&task_id, |t| {
                    t.message = last_status.clone();
                    t.progress = (10 + (progress as f64 * 0.9) as u8).min(100); // 10-100

                    // Update current layer info
                    if let Some(id) = &status.id {
                        t.current_layer = id.clone();
                    }
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
        t.current_layer = String::new();
    });
}

/// Estimate progress based on Docker API status
fn calculate_progress(status: &bollard::models::CreateImageInfo) -> u8 {
    // Simple estimate based on status keywords
    if let Some(ref s) = status.status {
        if s.contains("Pulling from") {
            return 15;
        } else if s.contains("Pulling fs layer") {
            return 20;
        } else if s.contains("Downloading") {
            // Try to calculate from progress detail
            if let Some(ref detail) = status.progress_detail {
                if let (Some(current), Some(total)) = (detail.current, detail.total) {
                    if total > 0 {
                        return (20 + (current as f64 / total as f64 * 60.0) as u64) as u8;
                    }
                }
            }
            return 80;
        } else if s.contains("Extracting") {
            if let Some(ref detail) = status.progress_detail {
                if let (Some(current), Some(total)) = (detail.current, detail.total) {
                    if total > 0 {
                        return (80 + (current as f64 / total as f64 * 15.0) as u64) as u8;
                    }
                }
            }
            return 95;
        } else if s.contains("Pull complete") || s.contains("Downloaded newer image") {
            return 95;
        } else if s.contains("Image is up to date") {
            return 100;
        }
    }
    50 // Default
}
