use actix_web::{delete, get, post, web, HttpRequest, HttpResponse, Responder};
use bollard::Docker;
use bollard::query_parameters::ListContainersOptions;
use serde::{Deserialize, Serialize};

use crate::DbPool;
use crate::utils::admin::{validate_token_with_db, verify_admin_access};

/// Container information
#[derive(Serialize, Debug)]
pub struct ContainerInfo {
    /// Container ID (short form)
    pub id: String,
    /// Container name (first name without leading slash)
    pub name: String,
    /// Image name
    pub image: String,
    /// Container status (e.g., "Up 2 hours", "Exited (0) 3 days ago")
    pub status: String,
    /// Created timestamp (Unix timestamp)
    pub created: i64,
}

/// Get containers response
#[derive(Serialize)]
pub struct GetContainersResponse {
    pub success: bool,
    pub message: String,
    pub containers: Vec<ContainerInfo>,
}

/// Format short ID from full ID
fn format_short_id(id: &str) -> String {
    id.chars().take(12).collect()
}

/// Get all containers (including offline and online)
///
/// # Endpoint
/// GET /docker/containers
///
/// # Response
/// ```json
/// {
///     "success": true,
///     "message": "Containers retrieved successfully",
///     "containers": [
///         {
///             "id": "abc123...",
///             "name": "my-container",
///             "image": "nginx:latest",
///             "status": "Up 2 hours",
///             "created": 1234567890
///         }
///     ]
/// }
/// ```
#[get("/docker/containers")]
pub async fn get_containers(req: HttpRequest, pool: web::Data<DbPool>) -> impl Responder {
    // 1. Verify admin access
    if let Err(response) = validate_token_with_db(&req, &pool).await {
        return response;
    }

    // 2. Connect to Docker
    let docker = match Docker::connect_with_local_defaults() {
        Ok(d) => d,
        Err(e) => {
            return HttpResponse::InternalServerError().json(GetContainersResponse {
                success: false,
                message: format!("Failed to connect to Docker: {}", e),
                containers: vec![],
            });
        }
    };

    // 3. List all containers (including stopped/offline ones)
    let options = Some(ListContainersOptions {
        all: true,
        ..Default::default()
    });

    match docker.list_containers(options).await {
        Ok(containers) => {
            let container_infos: Vec<ContainerInfo> = containers
                .into_iter()
                .map(|c| ContainerInfo {
                    id: format_short_id(&c.id.unwrap_or_default()),
                    name: c
                        .names
                        .as_ref()
                        .and_then(|n| n.first())
                        .map(|n| n.trim_start_matches('/').to_string())
                        .unwrap_or_default(),
                    image: c.image.unwrap_or_default(),
                    status: c.status.unwrap_or_default(),
                    created: c.created.unwrap_or_default(),
                })
                .collect();

            HttpResponse::Ok().json(GetContainersResponse {
                success: true,
                message: format!("Retrieved {} containers", container_infos.len()),
                containers: container_infos,
            })
        }
        Err(e) => HttpResponse::InternalServerError().json(GetContainersResponse {
            success: false,
            message: format!("Failed to list containers: {}", e),
            containers: vec![],
        }),
    }
}

/// Start container request
#[derive(Deserialize, Debug)]
pub struct StartContainerRequest {
    pub container_id: String,
}

/// Start container response
#[derive(Serialize)]
pub struct StartContainerResponse {
    pub success: bool,
    pub message: String,
}

/// Start a container
///
/// # Endpoint
/// POST /docker/container/start
///
/// # Request Body
/// ```json
/// {
///     "container_id": "abc123..."
/// }
/// ```
#[post("/docker/container/start")]
pub async fn start_container(
    req: HttpRequest,
    pool: web::Data<DbPool>,
    body: web::Json<StartContainerRequest>,
) -> impl Responder {
    // 1. Verify admin access
    let _admin_username = match verify_admin_access(&req, &pool).await {
        Ok(username) => username,
        Err(response) => return response,
    };

    let container_id = body.container_id.trim();

    if container_id.is_empty() {
        return HttpResponse::BadRequest().json(StartContainerResponse {
            success: false,
            message: "Container ID is required".to_string(),
        });
    }

    // 2. Connect to Docker
    let docker = match Docker::connect_with_local_defaults() {
        Ok(d) => d,
        Err(e) => {
            return HttpResponse::InternalServerError().json(StartContainerResponse {
                success: false,
                message: format!("Failed to connect to Docker: {}", e),
            });
        }
    };

    // 3. Start container
    match docker.start_container(container_id, None).await {
        Ok(_) => HttpResponse::Ok().json(StartContainerResponse {
            success: true,
            message: format!("Container '{}' started successfully", container_id),
        }),
        Err(e) => HttpResponse::InternalServerError().json(StartContainerResponse {
            success: false,
            message: format!("Failed to start container: {}", e),
        }),
    }
}

/// Stop container request
#[derive(Deserialize, Debug)]
pub struct StopContainerRequest {
    pub container_id: String,
    /// Timeout in seconds before killing the container
    #[serde(default = "default_stop_timeout")]
    pub timeout: i32,
}

fn default_stop_timeout() -> i32 {
    10
}

/// Stop container response
#[derive(Serialize)]
pub struct StopContainerResponse {
    pub success: bool,
    pub message: String,
}

/// Stop a container
///
/// # Endpoint
/// POST /docker/container/stop
///
/// # Request Body
/// ```json
/// {
///     "container_id": "abc123...",
///     "timeout": 10
/// }
/// ```
#[post("/docker/container/stop")]
pub async fn stop_container(
    req: HttpRequest,
    pool: web::Data<DbPool>,
    body: web::Json<StopContainerRequest>,
) -> impl Responder {
    // 1. Verify admin access
    let _admin_username = match verify_admin_access(&req, &pool).await {
        Ok(username) => username,
        Err(response) => return response,
    };

    let container_id = body.container_id.trim();

    if container_id.is_empty() {
        return HttpResponse::BadRequest().json(StopContainerResponse {
            success: false,
            message: "Container ID is required".to_string(),
        });
    }

    // 2. Connect to Docker
    let docker = match Docker::connect_with_local_defaults() {
        Ok(d) => d,
        Err(e) => {
            return HttpResponse::InternalServerError().json(StopContainerResponse {
                success: false,
                message: format!("Failed to connect to Docker: {}", e),
            });
        }
    };

    // 3. Stop container
    let options = bollard::query_parameters::StopContainerOptions {
        t: Some(body.timeout),
        ..Default::default()
    };

    match docker.stop_container(container_id, Some(options)).await {
        Ok(_) => HttpResponse::Ok().json(StopContainerResponse {
            success: true,
            message: format!("Container '{}' stopped successfully", container_id),
        }),
        Err(e) => HttpResponse::InternalServerError().json(StopContainerResponse {
            success: false,
            message: format!("Failed to stop container: {}", e),
        }),
    }
}

/// Restart container request
#[derive(Deserialize, Debug)]
pub struct RestartContainerRequest {
    pub container_id: String,
    /// Timeout in seconds before killing the container
    #[serde(default = "default_restart_timeout")]
    pub timeout: i32,
}

fn default_restart_timeout() -> i32 {
    10
}

/// Restart container response
#[derive(Serialize)]
pub struct RestartContainerResponse {
    pub success: bool,
    pub message: String,
}

/// Restart a container
///
/// # Endpoint
/// POST /docker/container/restart
///
/// # Request Body
/// ```json
/// {
///     "container_id": "abc123...",
///     "timeout": 10
/// }
/// ```
#[post("/docker/container/restart")]
pub async fn restart_container(
    req: HttpRequest,
    pool: web::Data<DbPool>,
    body: web::Json<RestartContainerRequest>,
) -> impl Responder {
    // 1. Verify admin access
    let _admin_username = match verify_admin_access(&req, &pool).await {
        Ok(username) => username,
        Err(response) => return response,
    };

    let container_id = body.container_id.trim();

    if container_id.is_empty() {
        return HttpResponse::BadRequest().json(RestartContainerResponse {
            success: false,
            message: "Container ID is required".to_string(),
        });
    }

    // 2. Connect to Docker
    let docker = match Docker::connect_with_local_defaults() {
        Ok(d) => d,
        Err(e) => {
            return HttpResponse::InternalServerError().json(RestartContainerResponse {
                success: false,
                message: format!("Failed to connect to Docker: {}", e),
            });
        }
    };

    // 3. Restart container
    let options = bollard::query_parameters::RestartContainerOptions {
        t: Some(body.timeout),
        ..Default::default()
    };

    match docker.restart_container(container_id, Some(options)).await {
        Ok(_) => HttpResponse::Ok().json(RestartContainerResponse {
            success: true,
            message: format!("Container '{}' restarted successfully", container_id),
        }),
        Err(e) => HttpResponse::InternalServerError().json(RestartContainerResponse {
            success: false,
            message: format!("Failed to restart container: {}", e),
        }),
    }
}

/// Remove container response
#[derive(Serialize)]
pub struct RemoveContainerResponse {
    pub success: bool,
    pub message: String,
}

/// Remove a container
///
/// # Endpoint
/// DELETE /docker/container/{container_id}
///
/// # Path Parameters
/// - `container_id`: Container ID (short or full)
#[delete("/docker/container/{container_id}")]
pub async fn remove_container(
    req: HttpRequest,
    pool: web::Data<DbPool>,
    path: web::Path<String>,
) -> impl Responder {
    // 1. Verify admin access
    let _admin_username = match verify_admin_access(&req, &pool).await {
        Ok(username) => username,
        Err(response) => return response,
    };

    let container_id = path.into_inner();

    if container_id.is_empty() {
        return HttpResponse::BadRequest().json(RemoveContainerResponse {
            success: false,
            message: "Container ID is required".to_string(),
        });
    }

    // 2. Connect to Docker
    let docker = match Docker::connect_with_local_defaults() {
        Ok(d) => d,
        Err(e) => {
            return HttpResponse::InternalServerError().json(RemoveContainerResponse {
                success: false,
                message: format!("Failed to connect to Docker: {}", e),
            });
        }
    };

    // 3. Remove container (force remove including running containers)
    let options = bollard::query_parameters::RemoveContainerOptions {
        force: true,
        v: true, // Remove volumes
        ..Default::default()
    };

    match docker.remove_container(&container_id, Some(options)).await {
        Ok(_) => HttpResponse::Ok().json(RemoveContainerResponse {
            success: true,
            message: format!("Container '{}' removed successfully", container_id),
        }),
        Err(e) => HttpResponse::InternalServerError().json(RemoveContainerResponse {
            success: false,
            message: format!("Failed to remove container: {}", e),
        }),
    }
}
