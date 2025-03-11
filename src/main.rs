use actix_web::{web, App, HttpResponse, HttpServer, Responder, HttpRequest};
use serde::{Deserialize, Serialize};
use std::fs;
use std::sync::Mutex;
use std::collections::HashMap;
use actix_web_actors::ws;
use actix::{Actor, StreamHandler, Handler, Message, AsyncContext};
use serde_json;
use serde_json::json;

/// 设备注册信息
#[derive(Debug, Deserialize, Serialize, Clone)]
struct Device {
    /// ESP8266 设备ID
    esp_id: String,
    /// 目标计算机MAC地址
    mac_address: String,
    /// 设备描述名称
    description: String,
    /// 密码
    password: String,
}

/// 唤醒请求
#[derive(Deserialize)]
struct WakeRequest {
    esp_id: String,
    password: String,
}

/// 设备数据存储
struct DeviceStore {
    devices: Mutex<HashMap<String, Device>>,
    file_path: String,
    active_connections: Mutex<HashMap<String, actix::Addr<WsConnection>>>,
}

impl DeviceStore {
    /// 创建新的设备存储实例
    fn new(file_path: &str) -> Self {
        if !std::path::Path::new(file_path).exists() {
            fs::write(file_path, "{}").expect("无法创建设备文件");
        }
        
        let devices = match fs::read_to_string(file_path) {
            Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
            Err(_) => HashMap::new(),
        };
        
        Self {
            devices: Mutex::new(devices),
            file_path: file_path.to_string(),
            active_connections: Mutex::new(HashMap::new()),
        }
    }

    /// 保存设备数据到文件
    fn save(&self) -> std::io::Result<()> {
        let devices = self.devices.lock().unwrap();
        let json = serde_json::to_string_pretty(&*devices)?;
        fs::write(&self.file_path, json)
    }
}

/// 注册新设备
async fn register_device(
    store: web::Data<DeviceStore>,
    device: web::Json<Device>,
) -> impl Responder {
    let mut devices = store.devices.lock().unwrap();
    devices.insert(device.esp_id.clone(), device.into_inner());
    
    match store.save() {
        Ok(_) => HttpResponse::Ok().json("设备注册成功"),
        Err(e) => HttpResponse::InternalServerError().body(e.to_string()),
    }
}

/// 获取所有注册设备
async fn get_devices(store: web::Data<DeviceStore>) -> impl Responder {
    let devices = match store.devices.lock() {
        Ok(guard) => guard,
        Err(_) => return HttpResponse::InternalServerError().json("获取设备列表失败"),
    };
    
    let devices_vec: Vec<Device> = devices.values().cloned().collect();
    
    HttpResponse::Ok()
        .insert_header(("Access-Control-Allow-Origin", "*"))
        .json(&devices_vec)
}

/// 发送唤醒命令到指定的 ESP8266
async fn wake_device(
    store: web::Data<DeviceStore>,
    wake_req: web::Json<WakeRequest>,
) -> impl Responder {
    let devices = store.devices.lock().unwrap();
    
    match devices.get(&wake_req.esp_id) {
        Some(device) => {
            if device.password != wake_req.password {
                return HttpResponse::Unauthorized().json("密码错误");
            }
            
            // 检查设备是否在线并发送唤醒命令
            let connections = store.active_connections.lock().unwrap();
            if let Some(addr) = connections.get(&wake_req.esp_id) {
                // 发送唤醒命令
                let wake_msg = json!({
                    "type": "wake",
                    "mac_address": device.mac_address
                });
                
                match addr.try_send(WsMessage(wake_msg.to_string())) {
                    Ok(_) => HttpResponse::Ok().json("唤醒命令已发送"),
                    Err(_) => HttpResponse::InternalServerError().json("发送唤醒命令失败"),
                }
            } else {
                HttpResponse::NotFound().json("设备不在线")
            }
        },
        None => HttpResponse::NotFound().json("设备未找到"),
    }
}

/// 首页处理程序
async fn index() -> impl Responder {
    HttpResponse::Ok().content_type("text/html").body(
        r#"
        <!DOCTYPE html>
        <html>
        <head>
            <title>WiFi 设备管理</title>
            <meta charset="utf-8">
            <style>
                body {
                    font-family: Arial, sans-serif;
                    max-width: 800px;
                    margin: 0 auto;
                    padding: 20px;
                }
                .device-card {
                    border: 1px solid #ddd;
                    padding: 15px;
                    margin: 10px 0;
                    border-radius: 5px;
                }
                .device-card:hover {
                    background-color: #f5f5f5;
                }
                .wake-btn {
                    background-color: #4CAF50;
                    color: white;
                    padding: 8px 16px;
                    border: none;
                    border-radius: 4px;
                    cursor: pointer;
                }
                .wake-btn:hover {
                    background-color: #45a049;
                }
                .status {
                    margin-top: 10px;
                    padding: 10px;
                    display: none;
                }
                .success {
                    background-color: #dff0d8;
                    color: #3c763d;
                    display: block;
                }
                .error {
                    background-color: #f2dede;
                    color: #a94442;
                    display: block;
                }
            </style>
        </head>
        <body>
            <h1>WiFi 设备管理系统</h1>
            <div id="status" class="status"></div>
            <div id="devices-container"></div>
            <div id="debug-info" style="margin-top: 20px; padding: 10px; background-color: #f0f0f0;"></div>

            <script>
                // 添加调试信息显示函数
                function showDebugInfo(info) {
                    const debugDiv = document.getElementById('debug-info');
                    debugDiv.innerHTML += `<p>${new Date().toLocaleTimeString()}: ${info}</p>`;
                }

                // 获取设备列表
                async function fetchDevices() {
                    try {
                        showDebugInfo('正在获取设备列表...');
                        const response = await fetch('/devices', {
                            method: 'GET',
                            headers: {
                                'Accept': 'application/json'
                            }
                        });
                        
                        showDebugInfo(`HTTP状态: ${response.status}`);
                        
                        if (!response.ok) {
                            throw new Error(`HTTP error! status: ${response.status}`);
                        }
                        
                        const responseText = await response.text();
                        showDebugInfo(`收到响应: ${responseText}`);
                        
                        const devices = JSON.parse(responseText);
                        showDebugInfo(`解析后的设备数量: ${devices.length}`);
                        
                        const container = document.getElementById('devices-container');
                        container.innerHTML = '';

                        if (!devices || devices.length === 0) {
                            container.innerHTML = '<p>暂无注册设备</p>';
                            return;
                        }

                        devices.forEach(device => {
                            showDebugInfo(`处理设备: ${JSON.stringify(device)}`);
                            const deviceElement = document.createElement('div');
                            deviceElement.className = 'device-card';
                            deviceElement.innerHTML = `
                                <h3>${device.description}</h3>
                                <p>设备ID: ${device.esp_id}</p>
                                <p>MAC地址: ${device.mac_address}</p>
                                <input type="password" id="pwd-${device.esp_id}" placeholder="输入设备密码">
                                <button class="wake-btn" onclick="wakeDevice('${device.esp_id}')">
                                    唤醒设备
                                </button>
                            `;
                            container.appendChild(deviceElement);
                        });
                    } catch (error) {
                        showDebugInfo(`错误: ${error.message}`);
                        showStatus('获取设备列表失败: ' + error.message, false);
                    }
                }

                // 唤醒设备
                async function wakeDevice(espId) {
                    try {
                        const passwordInput = document.getElementById(`pwd-${espId}`);
                        const password = passwordInput ? passwordInput.value : '';
                        
                        const response = await fetch('/wake', {
                            method: 'POST',
                            headers: {
                                'Content-Type': 'application/json',
                            },
                            body: JSON.stringify({ 
                                esp_id: espId,
                                password: password
                            })
                        });

                        if (response.ok) {
                            showStatus('唤醒命令已发送', true);
                        } else {
                            const error = await response.text();
                            showStatus('唤醒失败: ' + error, false);
                        }
                    } catch (error) {
                        showStatus('发送唤醒命令失败: ' + error.message, false);
                    }
                }

                // 显示状态信息
                function showStatus(message, isSuccess) {
                    const status = document.getElementById('status');
                    status.textContent = message;
                    status.className = 'status ' + (isSuccess ? 'success' : 'error');
                    setTimeout(() => {
                        status.className = 'status';
                    }, 3000);
                }

                // 页面加载时获取设备列表
                document.addEventListener('DOMContentLoaded', fetchDevices);

                // 每30秒刷新一次设备列表
                setInterval(fetchDevices, 30000);
            </script>
        </body>
        </html>
        "#
    )
}

/// WebSocket消息包装
#[derive(Message)]
#[rtype(result = "()")]
struct WsMessage(String);

/// WebSocket连接处理
struct WsConnection {
    esp_id: String,
    store: web::Data<DeviceStore>,
}

impl Handler<WsMessage> for WsConnection {
    type Result = ();

    fn handle(&mut self, msg: WsMessage, ctx: &mut Self::Context) {
        ctx.text(msg.0);
    }
}

impl Actor for WsConnection {
    type Context = ws::WebsocketContext<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        let mut connections = self.store.active_connections.lock().unwrap();
        connections.insert(self.esp_id.clone(), ctx.address());
    }

    fn stopped(&mut self, _ctx: &mut Self::Context) {
        let mut connections = self.store.active_connections.lock().unwrap();
        connections.remove(&self.esp_id);
    }
}

impl StreamHandler<Result<ws::Message, ws::ProtocolError>> for WsConnection {
    fn handle(&mut self, msg: Result<ws::Message, ws::ProtocolError>, ctx: &mut Self::Context) {
        match msg {
            Ok(ws::Message::Ping(msg)) => ctx.pong(&msg),
            Ok(ws::Message::Close(reason)) => {
                ctx.close(reason);
            },
            _ => (),
        }
    }
}

/// WebSocket连接处理函数
async fn ws_index(
    req: HttpRequest,
    stream: web::Payload,
    query: web::Query<HashMap<String, String>>,
    store: web::Data<DeviceStore>,
) -> Result<HttpResponse, actix_web::Error> {
    let esp_id = query.get("esp_id").cloned().unwrap_or_default();
    
    let ws = WsConnection { 
        esp_id, 
        store: store.clone()
    };
    
    ws::start(ws, &req, stream)
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let store = web::Data::new(DeviceStore::new("devices.json"));
    
    println!("服务器启动在 http://127.0.0.1:54001");

    HttpServer::new(move || {
        App::new()
            .app_data(store.clone())
            .route("/", web::get().to(index))
            .route("/register", web::post().to(register_device))
            .route("/devices", web::get().to(get_devices))
            .route("/wake", web::post().to(wake_device))
            .route("/ws", web::get().to(ws_index))
    })
    .bind("0.0.0.0:54001")?
    .run()
    .await
}
