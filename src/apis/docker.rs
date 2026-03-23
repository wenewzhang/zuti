use actix_web::{get, post, web, HttpRequest, HttpResponse, Responder};
use bollard::Docker;
use bollard::query_parameters::{CreateImageOptions, ListImagesOptions};
use futures_util::stream::StreamExt;
use serde::{Deserialize, Serialize};

use crate::utils::admin::verify_admin_access;
use crate::DbPool;

/// Docker 镜像信息结构体
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

/// 获取 Docker 镜像列表响应结构体
#[derive(Serialize)]
pub struct GetImagesResponse {
    pub success: bool,
    pub message: String,
    pub images: Vec<DockerImage>,
}

/// 拉取镜像请求结构体
#[derive(Deserialize)]
pub struct PullImageRequest {
    pub image_name: String,
}

/// 拉取镜像响应结构体
#[derive(Serialize)]
pub struct PullImageResponse {
    pub success: bool,
    pub message: String,
}

/// 获取 Docker 镜像列表（仅 admin 可访问）
///
/// # Endpoint
/// GET /docker/get_images
///
/// # Authorization
/// 需要 Bearer token，且用户类型必须为 admin
#[get("/docker/get_images")]
pub async fn get_images(req: HttpRequest, pool: web::Data<DbPool>) -> impl Responder {
    // 1. 验证 JWT token 并检查 admin 权限
    let _admin_username = match verify_admin_access(&req, &pool).await {
        Ok(username) => username,
        Err(response) => return response,
    };

    // 2. 连接 Docker
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

    // 3. 获取镜像列表
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

/// 拉取 Docker/Podman 镜像（仅 admin 可访问）
///
/// # Endpoint
/// POST /docker/pull_image
///
/// # Request Body
/// ```json
/// {
///     "image_name": "nginx:latest"
/// }
/// ```
///
/// # Authorization
/// 需要 Bearer token，且用户类型必须为 admin
#[post("/docker/pull_image")]
pub async fn pull_image(
    req: HttpRequest,
    pool: web::Data<DbPool>,
    body: web::Json<PullImageRequest>,
) -> impl Responder {
    // 1. 验证 JWT token 并检查 admin 权限
    let _admin_username = match verify_admin_access(&req, &pool).await {
        Ok(username) => username,
        Err(response) => return response,
    };

    // 2. 连接 Docker/Podman
    let docker = match Docker::connect_with_local_defaults() {
        Ok(docker) => docker,
        Err(e) => {
            return HttpResponse::InternalServerError().json(PullImageResponse {
                success: false,
                message: format!("Failed to connect to Docker/Podman: {}", e),
            });
        }
    };

    // 3. 解析镜像名称和标签
    let image_name = &body.image_name;
    let (repo, tag) = if image_name.contains(':') {
        let parts: Vec<&str> = image_name.splitn(2, ':').collect();
        (parts[0], parts[1])
    } else {
        (image_name.as_str(), "latest")
    };

    // 4. 创建拉取选项
    let options = Some(CreateImageOptions {
        from_image: Some(repo.to_string()),
        tag: Some(tag.to_string()),
        ..Default::default()
    });

    // 5. 执行拉取操作
    let mut stream = docker.create_image(options, None, None);
    let mut last_status = String::new();

    while let Some(result) = stream.next().await {
        match result {
            Ok(status) => {
                if let Some(status_str) = status.status {
                    last_status = status_str;
                }
                if let Some(error_detail) = status.error_detail {
                    return HttpResponse::InternalServerError().json(PullImageResponse {
                        success: false,
                        message: format!("Failed to pull image: {:?}", error_detail),
                    });
                }
            }
            Err(e) => {
                return HttpResponse::InternalServerError().json(PullImageResponse {
                    success: false,
                    message: format!("Failed to pull image: {}", e),
                });
            }
        }
    }

    HttpResponse::Ok().json(PullImageResponse {
        success: true,
        message: format!("Image '{}' pulled successfully: {}", image_name, last_status),
    })
}
