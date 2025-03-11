use actix_web::{web, App, HttpResponse, HttpServer, Responder, HttpRequest};
use serde::{Deserialize, Serialize};
use std::fs;
use std::sync::Mutex;
use std::collections::HashMap;
use actix_web_actors::ws;
use actix::{Actor, StreamHandler, Handler, Message, AsyncContext};
use serde_json;
use serde_json::json;

/// Device registration information
#[derive(Debug, Deserialize, Serialize, Clone)]
struct Device {
    /// ESP8266 Device ID
    esp_id: String,
    /// Target computer MAC address
    mac_address: String,
    /// Device description name
    description: String,
    /// Password
    password: String,
}

/// Wake request
#[derive(Deserialize)]
struct WakeRequest {
    esp_id: String,
    password: String,
}

/// Device data storage
struct DeviceStore {
    devices: Mutex<HashMap<String, Device>>,
    file_path: String,
    active_connections: Mutex<HashMap<String, actix::Addr<WsConnection>>>,
}

impl DeviceStore {
    /// Create a new device storage instance
    fn new(file_path: &str) -> Self {
        if !std::path::Path::new(file_path).exists() {
            fs::write(file_path, "{}").expect("Failed to create device file");
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

    /// Save device data to file
    fn save(&self) -> std::io::Result<()> {
        let json = {
            let devices = self.devices.lock().unwrap();
            serde_json::to_string_pretty(&*devices)?
        };
        fs::write(&self.file_path, json)
    }
}

/// Register new device
async fn register_device(
    store: web::Data<DeviceStore>,
    device: web::Json<Device>,
) -> impl Responder {
    println!("[Register] New device registration request: ID={}", device.esp_id);
    
    {
        let mut devices = store.devices.lock().unwrap();
        devices.insert(device.esp_id.clone(), device.into_inner());
    }
    
    match store.save() {
        Ok(_) => {
            println!("[Register] Device registered and saved successfully");
            HttpResponse::Ok().json("Device registered successfully")
        },
        Err(e) => {
            println!("[Register] Failed to save device info: {}", e);
            HttpResponse::InternalServerError().body(e.to_string())
        },
    }
}

/// Get all registered devices
async fn get_devices(store: web::Data<DeviceStore>) -> impl Responder {
    println!("[Query] Received request for device list");
    
    let devices_vec = {
        let devices = match store.devices.lock() {
            Ok(guard) => guard,
            Err(e) => {
                println!("[Query] Failed to get device list: {}", e);
                return HttpResponse::InternalServerError().json("Failed to get device list");
            }
        };
        devices.values().cloned().collect::<Vec<Device>>()
    };
    
    println!("[Query] Returning device list, total {} devices", devices_vec.len());
    
    HttpResponse::Ok()
        .insert_header(("Access-Control-Allow-Origin", "*"))
        .json(&devices_vec)
}

/// Send wake command to specified ESP8266
async fn wake_device(
    store: web::Data<DeviceStore>,
    wake_req: web::Json<WakeRequest>,
) -> impl Responder {
    println!("[Wake] Received wake request: ID={}", wake_req.esp_id);
    
    let device = {
        let devices = store.devices.lock().unwrap();
        devices.get(&wake_req.esp_id).cloned()
    };
    
    match device {
        Some(device) => {
            if device.password != wake_req.password {
                println!("[Wake] Password verification failed: ID={}", wake_req.esp_id);
                return HttpResponse::Unauthorized().json("Incorrect password");
            }
            
            let addr = {
                let connections = store.active_connections.lock().unwrap();
                connections.get(&wake_req.esp_id).cloned()
            };
            
            if let Some(addr) = addr {
                let wake_msg = json!({
                    "type": "wake",
                    "mac_address": device.mac_address
                });
                
                match addr.try_send(WsMessage(wake_msg.to_string())) {
                    Ok(_) => {
                        println!("[Wake] Wake command sent successfully: ID={}, MAC={}", wake_req.esp_id, device.mac_address);
                        HttpResponse::Ok().json("Wake command sent")
                    },
                    Err(e) => {
                        println!("[Wake] Failed to send wake command: {}", e);
                        HttpResponse::InternalServerError().json("Failed to send wake command")
                    },
                }
            } else {
                println!("[Wake] Device offline: ID={}", wake_req.esp_id);
                HttpResponse::NotFound().json("Device offline")
            }
        },
        None => {
            println!("[Wake] Device not found: ID={}", wake_req.esp_id);
            HttpResponse::NotFound().json("Device not found")
        },
    }
}

/// Home page handler
async fn index() -> impl Responder {
    HttpResponse::Ok().content_type("text/html").body(
        r#"
        <!DOCTYPE html>
        <html>
        <head>
            <title>WiFi Device Management</title>
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
            <h1>WiFi Device Management System</h1>
            <div id="status" class="status"></div>
            <div id="devices-container"></div>
            <div id="debug-info" style="margin-top: 20px; padding: 10px; background-color: #f0f0f0;"></div>

            <script>
                function showDebugInfo(info) {
                    const debugDiv = document.getElementById('debug-info');
                    debugDiv.innerHTML += `<p>${new Date().toLocaleTimeString()}: ${info}</p>`;
                }

                async function fetchDevices() {
                    try {
                        showDebugInfo('Fetching device list...');
                        const response = await fetch('/devices', {
                            method: 'GET',
                            headers: {
                                'Accept': 'application/json'
                            }
                        });
                        
                        showDebugInfo(`HTTP status: ${response.status}`);
                        
                        if (!response.ok) {
                            throw new Error(`HTTP error! status: ${response.status}`);
                        }
                        
                        const responseText = await response.text();
                        showDebugInfo(`Response received: ${responseText}`);
                        
                        const devices = JSON.parse(responseText);
                        showDebugInfo(`Parsed devices count: ${devices.length}`);
                        
                        const container = document.getElementById('devices-container');
                        container.innerHTML = '';

                        if (!devices || devices.length === 0) {
                            container.innerHTML = '<p>No registered devices</p>';
                            return;
                        }

                        devices.forEach(device => {
                            showDebugInfo(`Processing device: ${JSON.stringify(device)}`);
                            const deviceElement = document.createElement('div');
                            deviceElement.className = 'device-card';
                            deviceElement.innerHTML = `
                                <h3>${device.description}</h3>
                                <p>Device ID: ${device.esp_id}</p>
                                <p>MAC Address: ${device.mac_address}</p>
                                <input type="password" id="pwd-${device.esp_id}" placeholder="Enter device password">
                                <button class="wake-btn" onclick="wakeDevice('${device.esp_id}')">
                                    Wake Device
                                </button>
                            `;
                            container.appendChild(deviceElement);
                        });
                    } catch (error) {
                        showDebugInfo(`Error: ${error.message}`);
                        showStatus('Failed to fetch device list: ' + error.message, false);
                    }
                }

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
                            showStatus('Wake command sent', true);
                        } else {
                            const error = await response.text();
                            showStatus('Wake failed: ' + error, false);
                        }
                    } catch (error) {
                        showStatus('Failed to send wake command: ' + error.message, false);
                    }
                }

                function showStatus(message, isSuccess) {
                    const status = document.getElementById('status');
                    status.textContent = message;
                    status.className = 'status ' + (isSuccess ? 'success' : 'error');
                    setTimeout(() => {
                        status.className = 'status';
                    }, 3000);
                }

                document.addEventListener('DOMContentLoaded', fetchDevices);
                setInterval(fetchDevices, 30000);
            </script>
        </body>
        </html>
        "#
    )
}

/// WebSocket message wrapper
#[derive(Message)]
#[rtype(result = "()")]
struct WsMessage(String);

/// WebSocket connection handler
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
        println!("[WebSocket] New connection established: ID={}", self.esp_id);
        let mut connections = self.store.active_connections.lock().unwrap();
        connections.insert(self.esp_id.clone(), ctx.address());
    }

    fn stopped(&mut self, _ctx: &mut Self::Context) {
        println!("[WebSocket] Connection closed: ID={}", self.esp_id);
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

/// WebSocket connection handler function
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
    
    println!("[System] Server started at http://127.0.0.1:54001");
    println!("[System] WebSocket service is running");

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
