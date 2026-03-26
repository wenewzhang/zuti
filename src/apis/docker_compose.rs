use actix_web::{post, web, HttpRequest, HttpResponse, Responder};
use serde::{Deserialize, Serialize};
use serde_yaml::Value;
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::Command;

use crate::utils::admin::{validate_token_with_db, verify_admin_access};
use crate::utils::consts::FORBID_DIRECTORY;
use crate::DbPool;

const COMPOSE_DIR: &str = "/.data/zuti/compose";

#[derive(Deserialize, Debug)]
pub struct ComposeUpRequest {
    pub content: String,
    #[serde(default)]
    pub project_name: Option<String>,
    #[serde(default = "default_detached")]
    pub detached: bool,
    #[serde(default)]
    pub build: bool,
}

fn default_detached() -> bool {
    true
}

#[derive(Serialize)]
pub struct ComposeUpResponse {
    pub success: bool,
    pub message: String,
    pub project_name: Option<String>,
}

fn ensure_compose_dir() -> Result<(), String> {
    let path = Path::new(COMPOSE_DIR);
    if !path.exists() {
        fs::create_dir_all(path)
            .map_err(|e| format!("Failed to create compose directory: {}", e))?;
    }
    Ok(())
}

fn generate_project_name() -> String {
    format!("zuti-{}", uuid::Uuid::new_v4().to_string().split('-').next().unwrap())
}

/// Extract and validate host paths from volumes in compose YAML
fn validate_volumes(content: &str) -> Result<(), String> {
    let yaml: Value = serde_yaml::from_str(content)
        .map_err(|e| format!("Invalid YAML format: {}", e))?;

    // Get services section
    let services = yaml.get("services")
        .and_then(|v| v.as_mapping())
        .ok_or_else(|| "Missing or invalid 'services' section in YAML".to_string())?;

    for (service_name, service_config) in services {
        let service_name = service_name.as_str().unwrap_or("unknown");

        // Get volumes for this service
        if let Some(volumes) = service_config.get("volumes") {
            let volume_list = match volumes {
                Value::Sequence(seq) => seq,
                _ => {
                    return Err(format!("Service '{}': volumes must be an array", service_name));
                }
            };

            for (i, vol) in volume_list.iter().enumerate() {
                let host_path = match vol {
                    // Short syntax: "./host:/container" or "/host:/container"
                    // Anonymous volume: "/container" (no colon, skip validation)
                    Value::String(s) => {
                        if !s.contains(':') {
                            // Anonymous volume (temporary/cache data), no host path to validate
                            continue;
                        }
                        s.split(':').next().unwrap_or(s).to_string()
                    }
                    // Long syntax: { type: bind, source: /host, target: /container }
                    Value::Mapping(m) => {
                        if let Some(src) = m.get(&Value::String("source".to_string())) {
                            src.as_str().unwrap_or("").to_string()
                        } else {
                            continue; // No source, skip
                        }
                    }
                    _ => continue,
                };

                if host_path.is_empty() {
                    continue;
                }

                // Normalize path (remove trailing /)
                let normalized = host_path.trim_end_matches('/');

                // Check against forbidden directories
                for &forbidden in FORBID_DIRECTORY {
                    if normalized == forbidden || normalized.starts_with(&format!("{}/", forbidden)) {
                        return Err(format!(
                            "Service '{}': volume {} uses forbidden path '{}' (conflicts with '{}')",
                            service_name, i + 1, host_path, forbidden
                        ));
                    }
                }
            }
        }
    }

    Ok(())
}

fn write_compose_file(project_name: &str, content: &str) -> Result<std::path::PathBuf, String> {
    ensure_compose_dir()?;

    let project_dir = Path::new(COMPOSE_DIR).join(project_name);
    fs::create_dir_all(&project_dir)
        .map_err(|e| format!("Failed to create project directory: {}", e))?;

    let compose_path = project_dir.join("compose.yaml");
    let mut file = fs::File::create(&compose_path)
        .map_err(|e| format!("Failed to create compose file: {}", e))?;

    file.write_all(content.as_bytes())
        .map_err(|e| format!("Failed to write compose file: {}", e))?;

    Ok(compose_path)
}

fn execute_compose_up(
    project_name: &str,
    compose_path: &Path,
    detached: bool,
    build: bool,
) -> Result<String, String> {
    let check_cmd = Command::new("which")
        .arg("podman-compose")
        .output()
        .map_err(|e| format!("Failed to check podman-compose: {}", e))?;
    
    if !check_cmd.status.success() {
        return Err("podman-compose not found. Please install podman-compose first.".to_string());
    }
    
    let mut cmd = Command::new("podman-compose");
    cmd.arg("-f")
        .arg(compose_path)
        .arg("-p")
        .arg(project_name);
    
    if build {
        cmd.arg("--build");
    }
    
    cmd.arg("up");
    
    if detached {
        cmd.arg("-d");
    }
    
    let output = cmd
        .current_dir(compose_path.parent().unwrap())
        .output()
        .map_err(|e| format!("Failed to execute podman-compose: {}", e))?;
    
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    
    let result = if stderr.is_empty() { stdout.to_string() } else { format!("{}", stderr) };
    
    if output.status.success() {
        Ok(result)
    } else {
        Err(format!("podman-compose failed: {}", result))
    }
}

#[post("/docker/compose_up")]
pub async fn compose_up(
    req: HttpRequest,
    pool: web::Data<DbPool>,
    body: web::Json<ComposeUpRequest>,
) -> impl Responder {
    let _admin_username = match verify_admin_access(&req, &pool).await {
        Ok(username) => username,
        Err(response) => return response,
    };

    let req_body = body.into_inner();
    let content = req_body.content.trim();

    if content.is_empty() {
        return HttpResponse::BadRequest().json(ComposeUpResponse {
            success: false,
            message: "Compose file content is required".to_string(),
            project_name: None,
        });
    }

    // Validate volumes host paths against forbidden directories
    if let Err(e) = validate_volumes(content) {
        return HttpResponse::BadRequest().json(ComposeUpResponse {
            success: false,
            message: e,
            project_name: None,
        });
    }

    let project_name = req_body.project_name
        .filter(|n| !n.trim().is_empty())
        .map(|n| n.trim().to_string())
        .unwrap_or_else(generate_project_name);

    let compose_path = match write_compose_file(&project_name, content) {
        Ok(path) => path,
        Err(e) => {
            return HttpResponse::InternalServerError().json(ComposeUpResponse {
                success: false,
                message: e,
                project_name: Some(project_name),
            });
        }
    };

    match execute_compose_up(&project_name, &compose_path, req_body.detached, req_body.build) {
        Ok(_) => HttpResponse::Ok().json(ComposeUpResponse {
            success: true,
            message: format!("Compose project '{}' started successfully", project_name),
            project_name: Some(project_name),
        }),
        Err(e) => HttpResponse::InternalServerError().json(ComposeUpResponse {
            success: false,
            message: e,
            project_name: Some(project_name),
        }),
    }
}


// ============================================================================
// Compose List - List all compose projects
// ============================================================================

use actix_web::get;

/// Compose project info
#[derive(Serialize, Debug)]
pub struct ComposeProject {
    pub name: String,
    pub created_at: Option<u64>,
}

/// Compose list response
#[derive(Serialize)]
pub struct ComposeListResponse {
    pub success: bool,
    pub message: String,
    pub projects: Option<Vec<ComposeProject>>,
}

/// List all compose projects
///
/// # Endpoint
/// GET /docker/compose_list
///
/// # Response
/// ```json
/// {
///     "success": true,
///     "message": "Found 3 project(s)",
///     "projects": [
///         {"name": "nginx-demo", "created_at": 1704067200},
///         {"name": "myapp", "created_at": 1704067300}
///     ]
/// }
/// ```
#[get("/docker/compose_list")]
pub async fn compose_list(
    req: HttpRequest,
    pool: web::Data<DbPool>,
) -> impl Responder {
    // 1. Verify token (any valid user, not just admin)
    if let Err(response) = validate_token_with_db(&req, &pool).await {
        return response;
    }

    // 2. Read compose directory
    let compose_dir = Path::new(COMPOSE_DIR);
    
    if !compose_dir.exists() {
        return HttpResponse::Ok().json(ComposeListResponse {
            success: true,
            message: "No compose projects found".to_string(),
            projects: Some(vec![]),
        });
    }

    // 3. List all subdirectories (project names)
    let mut projects = Vec::new();
    
    match fs::read_dir(compose_dir) {
        Ok(entries) => {
            for entry in entries {
                if let Ok(entry) = entry {
                    let path = entry.path();
                    if path.is_dir() {
                        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                            // Get directory modification time as created_at
                            let created_at = entry.metadata()
                                .ok()
                                .and_then(|m| m.modified().ok())
                                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                                .map(|d| d.as_secs());
                            
                            projects.push(ComposeProject {
                                name: name.to_string(),
                                created_at,
                            });
                        }
                    }
                }
            }
        }
        Err(e) => {
            return HttpResponse::InternalServerError().json(ComposeListResponse {
                success: false,
                message: format!("Failed to read compose directory: {}", e),
                projects: None,
            });
        }
    }

    // Sort by name
    projects.sort_by(|a, b| a.name.cmp(&b.name));

    HttpResponse::Ok().json(ComposeListResponse {
        success: true,
        message: format!("Found {} project(s)", projects.len()),
        projects: Some(projects),
    })
}


// ============================================================================
// Compose Down - Stop and remove compose project
// ============================================================================

use actix_web::delete;

/// Compose down request
#[derive(Deserialize, Debug)]
pub struct ComposeDownRequest {
    pub project_name: String,
    /// Remove volumes (default: false)
    #[serde(default)]
    pub volumes: bool,
    /// Remove images (default: false)
    #[serde(default)]
    pub remove_images: bool,
}

/// Compose down response
#[derive(Serialize)]
pub struct ComposeDownResponse {
    pub success: bool,
    pub message: String,
}

/// Execute podman-compose down
fn execute_compose_down(
    project_name: &str,
    compose_path: &Path,
    volumes: bool,
    remove_images: bool,
) -> Result<String, String> {
    let check_cmd = Command::new("which")
        .arg("podman-compose")
        .output()
        .map_err(|e| format!("Failed to check podman-compose: {}", e))?;
    
    if !check_cmd.status.success() {
        return Err("podman-compose not found. Please install podman-compose first.".to_string());
    }
    
    let mut cmd = Command::new("podman-compose");
    cmd.arg("-f")
        .arg(compose_path)
        .arg("-p")
        .arg(project_name)
        .arg("down");
    
    if volumes {
        cmd.arg("-v");
    }
    
    if remove_images {
        cmd.arg("--rmi").arg("all");
    }
    
    let output = cmd
        .current_dir(compose_path.parent().unwrap())
        .output()
        .map_err(|e| format!("Failed to execute podman-compose down: {}", e))?;
    
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    
    let result = if stderr.is_empty() { stdout.to_string() } else { format!("{}", stderr) };
    
    if output.status.success() {
        Ok(result)
    } else {
        Err(format!("podman-compose down failed: {}", result))
    }
}

/// Delete compose project files (YAML only, not stopping containers)
///
/// # Endpoint
/// DELETE /docker/compose/delete
///
/// # Path Parameters
/// - `project_name`: Project name to delete
///
/// # Response
/// ```json
/// { "success": true, "message": "Project files deleted" }
/// ```
#[delete("/docker/compose/delete/{project_name}")]
pub async fn compose_delete(
    req: HttpRequest,
    pool: web::Data<DbPool>,
    path: web::Path<String>,
) -> impl Responder {
    // 1. Verify admin access
    let _admin_username = match verify_admin_access(&req, &pool).await {
        Ok(username) => username,
        Err(response) => return response,
    };

    // 2. Get project name from path
    let project_name = path.into_inner().trim().to_string();
    
    if project_name.is_empty() {
        return HttpResponse::BadRequest().json(ComposeDownResponse {
            success: false,
            message: "Project name is required".to_string(),
        });
    }

    // 3. Check if project exists
    let project_dir = Path::new(COMPOSE_DIR).join(&project_name);
    let compose_path = project_dir.join("compose.yaml");
    
    if !compose_path.exists() {
        return HttpResponse::NotFound().json(ComposeDownResponse {
            success: false,
            message: format!("Project '{}' not found", project_name),
        });
    }

    // 4. Remove project directory (YAML files only)
    match fs::remove_dir_all(&project_dir) {
        Ok(_) => HttpResponse::Ok().json(ComposeDownResponse {
            success: true,
            message: format!("Compose project '{}' files deleted successfully", project_name),
        }),
        Err(e) => HttpResponse::InternalServerError().json(ComposeDownResponse {
            success: false,
            message: format!("Failed to delete project files: {}", e),
        }),
    }
}


/// Stop and remove Docker/Podman Compose project
///
/// # Endpoint
/// DELETE /docker/compose_down
///
/// # Request Body
/// ```json
/// {
///     "project_name": "my-project",
///     "volumes": false,
///     "remove_images": false
/// }
/// ```
#[delete("/docker/compose_down")]
pub async fn compose_down(
    req: HttpRequest,
    pool: web::Data<DbPool>,
    body: web::Json<ComposeDownRequest>,
) -> impl Responder {
    // 1. Verify admin access
    let _admin_username = match verify_admin_access(&req, &pool).await {
        Ok(username) => username,
        Err(response) => return response,
    };

    // 2. Validate request
    let req_body = body.into_inner();
    let project_name = req_body.project_name.trim();
    
    if project_name.is_empty() {
        return HttpResponse::BadRequest().json(ComposeDownResponse {
            success: false,
            message: "Project name is required".to_string(),
        });
    }

    // 3. Check if project exists
    let project_dir = Path::new(COMPOSE_DIR).join(project_name);
    let compose_path = project_dir.join("compose.yaml");
    
    if !compose_path.exists() {
        return HttpResponse::NotFound().json(ComposeDownResponse {
            success: false,
            message: format!("Project '{}' not found", project_name),
        });
    }

    // 4. Execute podman-compose down
    match execute_compose_down(project_name, &compose_path, req_body.volumes, req_body.remove_images) {
        Ok(_) => {
            // 5. Remove project directory after successful down
            if let Err(e) = fs::remove_dir_all(&project_dir) {
                log::warn!("Failed to remove project directory: {}", e);
            }
            
            HttpResponse::Ok().json(ComposeDownResponse {
                success: true,
                message: format!("Compose project '{}' stopped and removed successfully", project_name),
            })
        }
        Err(e) => HttpResponse::InternalServerError().json(ComposeDownResponse {
            success: false,
            message: e,
        }),
    }
}


// ============================================================================
// Podman Pod Management APIs
// ============================================================================

use serde::ser::{SerializeStruct, Serializer};

/// Pod information
#[derive(Debug)]
pub struct PodInfo {
    pub id: String,
    pub name: String,
    pub status: String,
    pub created: String,
}

impl Serialize for PodInfo {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("PodInfo", 4)?;
        state.serialize_field("id", &self.id)?;
        state.serialize_field("name", &self.name)?;
        state.serialize_field("status", &self.status)?;
        state.serialize_field("created", &self.created)?;
        state.end()
    }
}

/// Pod list response
#[derive(Serialize)]
pub struct PodListResponse {
    pub success: bool,
    pub message: String,
    pub pods: Option<Vec<PodInfo>>,
}

/// Pod action response
#[derive(Serialize)]
pub struct PodActionResponse {
    pub success: bool,
    pub message: String,
}

/// Parse podman pod ps --format json output
fn parse_pod_ps_json(output: &str) -> Vec<PodInfo> {
    let mut pods = Vec::new();
    
    // podman pod ps --format json returns an array of objects
    if let Ok(json_value) = serde_json::from_str::<serde_json::Value>(output) {
        if let Some(array) = json_value.as_array() {
            for item in array {
                let id = item.get("Id")
                    .and_then(|v| v.as_str())
                    .map(|s| s.chars().take(12).collect())
                    .unwrap_or_default();
                
                let name = item.get("Name")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                
                let status = item.get("Status")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                
                let created = item.get("CreatedAt")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                
                pods.push(PodInfo {
                    id,
                    name,
                    status,
                    created,
                });
            }
        }
    }
    
    pods
}

/// List all pods
///
/// # Endpoint
/// GET /podman/pod/list
///
/// # Response
/// ```json
/// {
///     "success": true,
///     "message": "Found 2 pod(s)",
///     "pods": [
///         {"id": "abc123", "name": "my-pod", "status": "Running", "created": "2 hours ago"}
///     ]
/// }
/// ```
#[get("/podman/pod/list")]
pub async fn pod_list(
    req: HttpRequest,
    pool: web::Data<DbPool>,
) -> impl Responder {
    // 1. Verify token
    if let Err(response) = validate_token_with_db(&req, &pool).await {
        return response;
    }

    // 2. Check podman exists
    let check_cmd = Command::new("which")
        .arg("podman")
        .output();
    
    match check_cmd {
        Ok(output) if !output.status.success() => {
            return HttpResponse::ServiceUnavailable().json(PodListResponse {
                success: false,
                message: "podman not found. Please install podman first.".to_string(),
                pods: None,
            });
        }
        Err(e) => {
            return HttpResponse::InternalServerError().json(PodListResponse {
                success: false,
                message: format!("Failed to check podman: {}", e),
                pods: None,
            });
        }
        _ => {}
    }

    // 3. Execute podman pod ps --format json
    let output = match Command::new("podman")
        .args(["pod", "ps", "--format", "json"])
        .output()
    {
        Ok(output) => output,
        Err(e) => {
            return HttpResponse::InternalServerError().json(PodListResponse {
                success: false,
                message: format!("Failed to execute podman pod ps: {}", e),
                pods: None,
            });
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    if !output.status.success() {
        return HttpResponse::InternalServerError().json(PodListResponse {
            success: false,
            message: format!("podman pod ps failed: {}", stderr),
            pods: None,
        });
    }

    // 4. Parse JSON output
    let pods = parse_pod_ps_json(&stdout);
    let count = pods.len();

    HttpResponse::Ok().json(PodListResponse {
        success: true,
        message: format!("Found {} pod(s)", count),
        pods: Some(pods),
    })
}


/// Start a pod
///
/// # Endpoint
/// POST /podman/pod/start/{name_or_id}
///
/// # Response
/// ```json
/// { "success": true, "message": "Pod 'my-pod' started successfully" }
/// ```
#[post("/podman/pod/start/{name_or_id}")]
pub async fn pod_start(
    req: HttpRequest,
    pool: web::Data<DbPool>,
    path: web::Path<String>,
) -> impl Responder {
    // 1. Verify admin access
    let _admin_username = match verify_admin_access(&req, &pool).await {
        Ok(username) => username,
        Err(response) => return response,
    };

    let name_or_id = path.into_inner();

    if name_or_id.is_empty() {
        return HttpResponse::BadRequest().json(PodActionResponse {
            success: false,
            message: "Pod name or ID is required".to_string(),
        });
    }

    // 2. Execute podman pod start
    let output = match Command::new("podman")
        .args(["pod", "start", &name_or_id])
        .output()
    {
        Ok(output) => output,
        Err(e) => {
            return HttpResponse::InternalServerError().json(PodActionResponse {
                success: false,
                message: format!("Failed to execute podman pod start: {}", e),
            });
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    if output.status.success() {
        HttpResponse::Ok().json(PodActionResponse {
            success: true,
            message: format!("Pod '{}' started successfully: {}", name_or_id, stdout.trim()),
        })
    } else {
        HttpResponse::InternalServerError().json(PodActionResponse {
            success: false,
            message: format!("Failed to start pod '{}': {}", name_or_id, stderr),
        })
    }
}


/// Stop a pod
///
/// # Endpoint
/// POST /podman/pod/stop/{name_or_id}
///
/// # Response
/// ```json
/// { "success": true, "message": "Pod 'my-pod' stopped successfully" }
/// ```
#[post("/podman/pod/stop/{name_or_id}")]
pub async fn pod_stop(
    req: HttpRequest,
    pool: web::Data<DbPool>,
    path: web::Path<String>,
) -> impl Responder {
    // 1. Verify admin access
    let _admin_username = match verify_admin_access(&req, &pool).await {
        Ok(username) => username,
        Err(response) => return response,
    };

    let name_or_id = path.into_inner();

    if name_or_id.is_empty() {
        return HttpResponse::BadRequest().json(PodActionResponse {
            success: false,
            message: "Pod name or ID is required".to_string(),
        });
    }

    // 2. Execute podman pod stop
    let output = match Command::new("podman")
        .args(["pod", "stop", &name_or_id])
        .output()
    {
        Ok(output) => output,
        Err(e) => {
            return HttpResponse::InternalServerError().json(PodActionResponse {
                success: false,
                message: format!("Failed to execute podman pod stop: {}", e),
            });
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    if output.status.success() {
        HttpResponse::Ok().json(PodActionResponse {
            success: true,
            message: format!("Pod '{}' stopped successfully: {}", name_or_id, stdout.trim()),
        })
    } else {
        HttpResponse::InternalServerError().json(PodActionResponse {
            success: false,
            message: format!("Failed to stop pod '{}': {}", name_or_id, stderr),
        })
    }
}


/// Restart a pod
///
/// # Endpoint
/// POST /podman/pod/restart/{name_or_id}
///
/// # Response
/// ```json
/// { "success": true, "message": "Pod 'my-pod' restarted successfully" }
/// ```
#[post("/podman/pod/restart/{name_or_id}")]
pub async fn pod_restart(
    req: HttpRequest,
    pool: web::Data<DbPool>,
    path: web::Path<String>,
) -> impl Responder {
    // 1. Verify admin access
    let _admin_username = match verify_admin_access(&req, &pool).await {
        Ok(username) => username,
        Err(response) => return response,
    };

    let name_or_id = path.into_inner();

    if name_or_id.is_empty() {
        return HttpResponse::BadRequest().json(PodActionResponse {
            success: false,
            message: "Pod name or ID is required".to_string(),
        });
    }

    // 2. Execute podman pod restart
    let output = match Command::new("podman")
        .args(["pod", "restart", &name_or_id])
        .output()
    {
        Ok(output) => output,
        Err(e) => {
            return HttpResponse::InternalServerError().json(PodActionResponse {
                success: false,
                message: format!("Failed to execute podman pod restart: {}", e),
            });
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    if output.status.success() {
        HttpResponse::Ok().json(PodActionResponse {
            success: true,
            message: format!("Pod '{}' restarted successfully: {}", name_or_id, stdout.trim()),
        })
    } else {
        HttpResponse::InternalServerError().json(PodActionResponse {
            success: false,
            message: format!("Failed to restart pod '{}': {}", name_or_id, stderr),
        })
    }
}


/// Remove a pod
///
/// # Endpoint
/// DELETE /podman/pod/remove/{name_or_id}
///
/// # Response
/// ```json
/// { "success": true, "message": "Pod 'my-pod' removed successfully" }
/// ```
#[delete("/podman/pod/remove/{name_or_id}")]
pub async fn pod_remove(
    req: HttpRequest,
    pool: web::Data<DbPool>,
    path: web::Path<String>,
) -> impl Responder {
    // 1. Verify admin access
    let _admin_username = match verify_admin_access(&req, &pool).await {
        Ok(username) => username,
        Err(response) => return response,
    };

    let name_or_id = path.into_inner();

    if name_or_id.is_empty() {
        return HttpResponse::BadRequest().json(PodActionResponse {
            success: false,
            message: "Pod name or ID is required".to_string(),
        });
    }

    // 2. Execute podman pod rm --force
    let output = match Command::new("podman")
        .args(["pod", "rm", &name_or_id])
        .output()
    {
        Ok(output) => output,
        Err(e) => {
            return HttpResponse::InternalServerError().json(PodActionResponse {
                success: false,
                message: format!("Failed to execute podman pod rm: {}", e),
            });
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    if output.status.success() {
        HttpResponse::Ok().json(PodActionResponse {
            success: true,
            message: format!("Pod '{}' removed successfully: {}", name_or_id, stdout.trim()),
        })
    } else {
        HttpResponse::InternalServerError().json(PodActionResponse {
            success: false,
            message: format!("Failed to remove pod '{}': {}", name_or_id, stderr),
        })
    }
}
