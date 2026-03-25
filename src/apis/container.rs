use actix_web::{delete, get, post, web, HttpRequest, HttpResponse, Responder};
use bollard::Docker;
use bollard::query_parameters::ListContainersOptions;
use serde::{Deserialize, Serialize};

use crate::utils::admin::verify_admin_access;
use crate::DbPool;

/// Helper trait to convert enum to string
trait ToStringOpt {
    fn to_string_opt(self) -> String;
}

impl ToStringOpt for Option<bollard::models::ContainerSummaryStateEnum> {
    fn to_string_opt(self) -> String {
        self.map(|s| format!("{:?}", s).to_lowercase())
            .unwrap_or_default()
    }
}

impl ToStringOpt for Option<bollard::models::PortSummaryTypeEnum> {
    fn to_string_opt(self) -> String {
        self.map(|s| format!("{:?}", s).to_lowercase())
            .unwrap_or_default()
    }
}

impl ToStringOpt for Option<bollard::models::MountPointTypeEnum> {
    fn to_string_opt(self) -> String {
        self.map(|s| format!("{:?}", s).to_lowercase())
            .unwrap_or_default()
    }
}

/// Container information
#[derive(Serialize, Debug)]
pub struct ContainerInfo {
    /// Container ID (short form)
    pub id: String,
    /// Container names (with leading slash)
    pub names: Vec<String>,
    /// Image name
    pub image: String,
    /// Image ID
    pub image_id: String,
    /// Command
    pub command: String,
    /// Created timestamp (Unix timestamp)
    pub created: i64,
    /// Container status (e.g., "Up 2 hours", "Exited (0) 3 days ago")
    pub status: String,
    /// Container state (e.g., "running", "exited", "paused", "restarting", "dead")
    pub state: String,
    /// Ports
    pub ports: Vec<PortMapping>,
    /// Labels
    pub labels: std::collections::HashMap<String, String>,
    /// Size of the container's root filesystem (RW)
    pub size_rw: Option<i64>,
    /// Size of the container's root filesystem (total)
    pub size_root_fs: Option<i64>,
    /// Host config
    pub host_config: Option<HostConfigInfo>,
    /// Network settings
    pub network_settings: Option<NetworkSettingsInfo>,
    /// Mounts
    pub mounts: Vec<MountInfo>,
}

/// Port mapping information
#[derive(Serialize, Debug)]
pub struct PortMapping {
    pub ip: String,
    pub private_port: u16,
    pub public_port: Option<u16>,
    pub typ: String,
}

/// Host config info
#[derive(Serialize, Debug)]
pub struct HostConfigInfo {
    pub network_mode: String,
}

/// Network settings info
#[derive(Serialize, Debug)]
pub struct NetworkSettingsInfo {
    pub networks: std::collections::HashMap<String, NetworkInfo>,
}

/// Network info
#[derive(Serialize, Debug)]
pub struct NetworkInfo {
    pub ip_address: String,
    pub gateway: String,
    pub mac_address: String,
}

/// Mount info
#[derive(Serialize, Debug)]
pub struct MountInfo {
    pub typ: String,
    pub source: String,
    pub destination: String,
    pub mode: String,
    pub rw: bool,
    pub propagation: String,
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
/// GET /docker/container
///
/// # Response
/// ```json
/// {
///     "success": true,
///     "message": "Containers retrieved successfully",
///     "containers": [
///         {
///             "id": "abc123...",
///             "names": ["/my-container"],
///             "image": "nginx:latest",
///             "image_id": "sha256:xxx...",
///             "command": "nginx -g 'daemon off;'",
///             "created": 1234567890,
///             "status": "Up 2 hours",
///             "state": "running",
///             "ports": [...],
///             "labels": {...},
///             ...
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
                    names: c.names.unwrap_or_default(),
                    image: c.image.unwrap_or_default(),
                    image_id: format_short_id(&c.image_id.unwrap_or_default()),
                    command: c.command.unwrap_or_default(),
                    created: c.created.unwrap_or_default(),
                    status: c.status.unwrap_or_default(),
                    state: c.state.to_string_opt(),
                    ports: c
                        .ports
                        .unwrap_or_default()
                        .into_iter()
                        .map(|p| PortMapping {
                            ip: p.ip.unwrap_or_default(),
                            private_port: p.private_port,
                            public_port: p.public_port,
                            typ: p.typ.to_string_opt(),
                        })
                        .collect(),
                    labels: c.labels.unwrap_or_default(),
                    size_rw: c.size_rw,
                    size_root_fs: c.size_root_fs,
                    host_config: c.host_config.map(|h| HostConfigInfo {
                        network_mode: h.network_mode.unwrap_or_default(),
                    }),
                    network_settings: c.network_settings.map(|n| NetworkSettingsInfo {
                        networks: n
                            .networks
                            .unwrap_or_default()
                            .into_iter()
                            .map(|(name, net)| {
                                (
                                    name,
                                    NetworkInfo {
                                        ip_address: net.ip_address.unwrap_or_default(),
                                        gateway: net.gateway.unwrap_or_default(),
                                        mac_address: net.mac_address.unwrap_or_default(),
                                    },
                                )
                            })
                            .collect(),
                    }),
                    mounts: c
                        .mounts
                        .unwrap_or_default()
                        .into_iter()
                        .map(|m| MountInfo {
                            typ: m.typ.to_string_opt(),
                            source: m.source.unwrap_or_default(),
                            destination: m.destination.unwrap_or_default(),
                            mode: m.mode.unwrap_or_default(),
                            rw: m.rw.unwrap_or_default(),
                            propagation: m.propagation.unwrap_or_default(),
                        })
                        .collect(),
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
