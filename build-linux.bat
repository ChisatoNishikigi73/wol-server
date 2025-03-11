@echo off
chcp 65001
echo 正在开始交叉编译 Linux (x86_64) 版本...

:: 检查 Docker 状态
docker info > nul 2>&1
if %errorlevel% neq 0 (
    echo 错误：Docker 未运行！
    echo 请先启动 Docker Desktop
    exit /b 1
)

:: 安装必要工具
echo 正在安装必要工具...
rustup target add x86_64-unknown-linux-musl
cargo install cross --force

:: 清理旧的构建
echo 清理旧的构建...
cargo clean

:: 开始交叉编译
echo 开始编译...
cross build --release --target x86_64-unknown-linux-musl

:: 检查编译结果
if %errorlevel% neq 0 (
    echo 编译失败！
    exit /b 1
)

:: 创建发布目录
echo 创建发布目录...
if not exist "release\linux" mkdir release\linux

:: 复制文件
echo 复制文件...
copy "target\x86_64-unknown-linux-musl\release\wol-server" "release\linux\"
if %errorlevel% neq 0 (
    echo 复制文件失败！
    exit /b 1
)

echo 构建完成！
echo 二进制文件位于：release\linux\wol-server 