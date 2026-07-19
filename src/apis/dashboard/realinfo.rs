use actix_web::{get, web, HttpRequest, HttpResponse, Responder};
use lazy_static::lazy_static;
use serde::Serialize;
use std::fs;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::utils::admin::validate_token_with_db;
use crate::DbPool;

#[derive(Serialize)]
pub struct CpuInfo {
    pub usage: f64,
    pub temp: i32,
    pub load: Vec<f64>,
}

#[derive(Serialize)]
pub struct MemoryInfo {
    pub total: u64,
    pub used: u64,
}

#[derive(Serialize)]
pub struct NetworkInfo {
    pub rx_rate: u64,
    pub tx_rate: u64,
}

#[derive(Serialize)]
pub struct RealInfoResponse {
    pub cpu: CpuInfo,
    pub memory: MemoryInfo,
    pub network: NetworkInfo,
}

struct Sample {
    timestamp: u64,
    cpu_total: u64,
    cpu_idle: u64,
    net_rx: u64,
    net_tx: u64,
}

lazy_static! {
    static ref LAST_SAMPLE: Mutex<Option<Sample>> = Mutex::new(None);
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn read_cpu_stat() -> Option<(u64, u64)> {
    let content = fs::read_to_string("/proc/stat").ok()?;
    let line = content.lines().next()?;
    if !line.starts_with("cpu ") {
        return None;
    }
    let values: Vec<u64> = line
        .split_whitespace()
        .skip(1)
        .filter_map(|s| s.parse().ok())
        .collect();
    if values.len() < 4 {
        return None;
    }
    let total: u64 = values.iter().sum();
    let idle = values[3];
    Some((total, idle))
}

fn read_cpu_temp() -> i32 {
    if let Ok(entries) = fs::read_dir("/sys/class/hwmon") {
        for entry in entries.flatten() {
            let path = entry.path();
            let name = fs::read_to_string(path.join("name"))
                .unwrap_or_default()
                .trim()
                .to_string();
            if name == "coretemp" || name == "x86_pkg_temp" || name == "k10temp" || name == "zenpower" {
                for i in 1..=20 {
                    if let Ok(s) = fs::read_to_string(path.join(format!("temp{}_input", i))) {
                        if let Ok(milli) = s.trim().parse::<i32>() {
                            return milli / 1000;
                        }
                    }
                }
            }
        }
    }

    if let Ok(entries) = fs::read_dir("/sys/class/thermal") {
        for entry in entries.flatten() {
            let path = entry.path();
            if path
                .file_name()
                .map_or(false, |n| n.to_string_lossy().starts_with("thermal_zone"))
            {
                if let Ok(s) = fs::read_to_string(path.join("temp")) {
                    if let Ok(milli) = s.trim().parse::<i32>() {
                        return milli / 1000;
                    }
                }
            }
        }
    }

    0
}

fn read_load_avg() -> Vec<f64> {
    fs::read_to_string("/proc/loadavg")
        .unwrap_or_default()
        .split_whitespace()
        .take(3)
        .filter_map(|s| s.parse().ok())
        .collect()
}

fn read_memory_info() -> MemoryInfo {
    let mut total = 0u64;
    let mut available = 0u64;

    if let Ok(content) = fs::read_to_string("/proc/meminfo") {
        for line in content.lines() {
            let mut parts = line.split_whitespace();
            let key = parts.next().unwrap_or("");
            let value = parts.next().unwrap_or("0");
            let val = value.parse::<u64>().unwrap_or(0) * 1024;
            match key {
                "MemTotal:" => total = val,
                "MemAvailable:" => available = val,
                _ => {}
            }
        }
    }

    MemoryInfo {
        total,
        used: total.saturating_sub(available),
    }
}

fn read_net_counters() -> (u64, u64) {
    let mut rx = 0u64;
    let mut tx = 0u64;

    if let Ok(content) = fs::read_to_string("/proc/net/dev") {
        for line in content.lines().skip(2) {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 10 {
                continue;
            }
            let iface = parts[0].trim_end_matches(':');
            if iface == "lo" {
                continue;
            }
            if let (Ok(r), Ok(t)) = (parts[1].parse::<u64>(), parts[9].parse::<u64>()) {
                rx += r;
                tx += t;
            }
        }
    }

    (rx, tx)
}

fn build_response() -> RealInfoResponse {
    let now_ts = now_secs();
    let (cpu_total, cpu_idle) = read_cpu_stat().unwrap_or((0, 0));
    let memory = read_memory_info();
    let (net_rx, net_tx) = read_net_counters();

    let mut guard = LAST_SAMPLE.lock().unwrap_or_else(|e| e.into_inner());

    let (cpu_usage, rx_rate, tx_rate) = match guard.as_ref() {
        Some(prev) if prev.timestamp < now_ts => {
            let cpu_usage = if cpu_total > prev.cpu_total {
                let total_diff = cpu_total - prev.cpu_total;
                let idle_diff = cpu_idle.saturating_sub(prev.cpu_idle);
                ((total_diff - idle_diff) as f64 / total_diff as f64) * 100.0
            } else {
                0.0
            };
            let dt = now_ts - prev.timestamp;
            let rx_rate = if dt > 0 {
                net_rx.saturating_sub(prev.net_rx) / dt
            } else {
                0
            };
            let tx_rate = if dt > 0 {
                net_tx.saturating_sub(prev.net_tx) / dt
            } else {
                0
            };
            (cpu_usage.max(0.0), rx_rate, tx_rate)
        }
        _ => (0.0, 0, 0),
    };

    *guard = Some(Sample {
        timestamp: now_ts,
        cpu_total,
        cpu_idle,
        net_rx,
        net_tx,
    });

    RealInfoResponse {
        cpu: CpuInfo {
            usage: (cpu_usage * 10.0).round() / 10.0,
            temp: read_cpu_temp(),
            load: read_load_avg(),
        },
        memory,
        network: NetworkInfo { rx_rate, tx_rate },
    }
}

/// GET /dashboard/realinfo - 返回系统实时信息（需要 JWT 认证）
#[get("/dashboard/realinfo")]
pub async fn get_realinfo(req: HttpRequest, pool: web::Data<DbPool>) -> impl Responder {
    if let Err(response) = validate_token_with_db(&req, &pool).await {
        return response;
    }

    HttpResponse::Ok().json(build_response())
}
