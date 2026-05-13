# Roon Beacon

[English](README.md) | 简体中文

把远端网络的 Roon Core 信标广播到本地局域网，让本地 Roon Remote / Control 客户端可以跨网段发现并使用它。

## 技术实现

SOOD 协议的二进制解析、序列化、常量定义直接使用 [roon-rs](https://github.com/shin1ohno/roon-rs) 项目的 `roon-sood` crate，复制到本地 `crates/roon-sood/` 作为 workspace member。roon-beacon 自身只做桥接逻辑（SOOD 代理、TCP 转发、域名解析），不重新实现协议编解码。

## 它解决的问题

Roon 原生使用 SOOD（一种基于 UDP 多播 `239.255.90.90:9003` 的协议）做 Core 发现。多播只在同一个二层局域网生效，一旦 Core 在另一个网段（比如 NAS 通过 VPN / Tailscale 接入），Remote 就发现不了 Core。

Roon Beacon 运行在**客户端所在的局域网**里，作为一个"SOOD 信标 + TCP 端口转发器"存在：

```
                        本地局域网  192.168.x.x                            远端网络  10.x.x.x
┌─────────────┐         ┌──────────────────────────────┐           ┌──────────────────────────┐
│ Roon Remote │ ──SOOD──▶  Roon Beacon                  │ ──TCP───▶ │  Roon Core (on NAS)      │
│ (手机/电脑) │ ◀─响应──   (绑 192.168.x.x)              │ ◀──TCP─── │  (监听 9100-9332/55000)  │
└─────────────┘         │  - 发现应答（SOOD）          │           └──────────────────────────┘
                        │  - TCP 转发 9100/9150/...    │
                        └──────────────────────────────┘
                                   │
                                   └── 通过 VPN / Tailscale / 端口映射 等方式连通
```

它做的事：
1. **SOOD 应答**：监听本地多播 `239.255.90.90:9003`，收到 Remote 的查询时**实时透传**给远端 Core，用 Core 真实响应的字节回给 Remote（保证 `service_id / unique_id / config_version` 等字段绝对正确）
2. **TCP 转发**：监听本机局域网 IP 上的所有 Roon 关键端口（9100 / 9101 / 9150 / 9330 / 9331 / 9332 / 55000），双向转发给远端 Core
3. **可选反向代理**：把本地 RAAT Bridge 的多播广告转给远端 Core，让 Core 在设备列表能看到本地音频设备

## 快速开始

### 前置条件

- Rust 1.70+ / Cargo
- 本机有一张网卡在客户端所在的局域网（比如 `192.168.1.x`）
- 本机能通过某种方式（VPN / Tailscale / 端口映射）直连到远端 Core 的 UDP 9003 和 TCP 9100-9332 / 55000
- 远端的 Roon Core / Roon Server 已经在运行

### 编译

```bash
cargo build --release
```

产物在 `target/release/roon-beacon`。

项目是 Cargo workspace，包含两个 crate：
- `crates/roon-sood` — SOOD 协议库，来自 [roon-rs](https://github.com/shin1ohno/roon-rs)，提供 `parse` / `serialize_query` / `serialize_response` 及常量定义
- `crates/roon-beacon` — 桥接主程序

### 最小用法

```bash
./target/release/roon-beacon -r <远端Core地址> -l <本机局域网IP>
```

例如：
```bash
./target/release/roon-beacon -r 100.107.146.8 -l 192.168.1.10
```

- `-r / --remote-ip`：远端 Core 的地址（IP / 主机名 / 域名都可以）
- `-l / --local-ip`：本机在局域网里的 IP（**不要**填 VPN / overlay 网络的 IP）

Linux 上监听 UDP 9003 通常需要 `sudo`。

### 在手机 / Remote 上使用

1. 启动 Roon Beacon 后，在手机（与 Roon Beacon 同一局域网）上打开 Roon Remote
2. 正常等待自动发现，或手动输入 `<本机局域网IP>` 连接
3. Remote 会把它当作 Core 使用，所有请求转发给远端真 Core

### Web UI

程序内置一个简洁的 Web UI，默认地址：

```text
http://127.0.0.1:9339
```

Web UI 支持英文 / 中文切换，可以查看运行状态和计数器，并修改不需要重新绑定端口的运行时设置，例如兜底显示名、主动广播间隔、实时 SOOD 超时和反向代理设置。

## 参数完整说明

```
-r, --remote-ip <HOST>        远端 Core 地址，支持 IP / 主机名 / 域名
-l, --local-ip  <IP>          本机局域网 IP（客户端所在的网段）
-n, --name      <NAME>        Core 显示名（默认 "Roon Core (Beacon)"）
    --prefer    <v4|v6|any>   域名解析时的地址族偏好（默认 v4）
    --broadcast-interval <N>  主动广播 SOOD 心跳间隔秒（默认 10，0=禁用）
    --use-broadcast           用 UDP 广播 255.255.255.255 代替多播
                              （路由器不支持 IGMP 多播时用）
    --retry-secs <N>          TCP 连接失败重试间隔（默认 3）
    --skip-ports <P1,P2>      跳过某些端口（端口已被其他程序占用时）
    --no-probe                禁用启动时向 Core 探测字段模板
    --probe-timeout <N>       探测超时秒（默认 3）
    --reverse-proxy           启用反向 SOOD 代理（把本地 RAAT 广告转给 Core）
    --proxy-ip <IP>           反向代理改写广告用的 IP（桥接对 Core 可达的 IP）
```

## 常见场景

### 场景 1：NAS + Tailscale

NAS 装了 Roon Server + Tailscale，客户端电脑同样装了 Tailscale，但客户端和 NAS 不在同一局域网。

```bash
# 在客户端（例如 192.168.1.10）上跑
./target/release/roon-beacon -r 100.107.146.8 -l 192.168.1.10
```

`100.107.146.8` 是 NAS 的 Tailscale IP。手机不需要装 Tailscale，只要和客户端同一 WiFi 就能用。

### 场景 2：NAS 做了端口映射到公网

```bash
./target/release/roon-beacon -r nas.example.com -l 192.168.1.10 --prefer v4
```

`--prefer v4` 强制用 IPv4 解析，避免走不通的 IPv6。

### 场景 3：本机也在跑 Roon Bridge / RAATServer

官方 Roon Bridge 可能占用 9200。Roon Beacon 会自动检测端口占用并跳过，你无需干预。

### 场景 4：路由器屏蔽多播

路由器不支持 IGMP 多播时，加 `--use-broadcast` 使用 UDP 广播代替。

## 故障排查

### 手机发现不了 Core

1. 确认手机和运行 Beacon 的机器在**同一 WiFi / 同一局域网**
2. 检查 Beacon 启动日志里有没有类似 `✓ TCP 192.168.x.x:9330 → ...` 的条目 —— 没有说明端口被占了
3. 看有没有 `🔍 SOOD 查询 from 192.168.x.x` 日志 —— 没有说明手机没在发 SOOD 扫描（可能 Roon app 用了缓存）
4. 手动连接试试：Roon Remote → 设置 → Roon Core → 手动连接 → 输入 `<本机局域网IP>`
5. 清手机 Roon Remote 缓存再试

### Core 探测超时

```
探测超时（3s），尝试下一个 service_id
```

可能原因：
- 远端 Core 没在运行（到 NAS 上看 Roon Server 服务）
- VPN / Tailscale 链路不通（`ping <远端IP>` 测一下）
- 防火墙拦住了 UDP 9003

你也可以用 `--no-probe` 跳过探测，用内置字段模板兜底（但字段可能不够，Remote 可能识别不了）。

### 端口被占用

日志里会看到：
```
⏭  跳过 TCP :9200  [Roon Web Display]（已被占用: Address already in use）
   这通常是本机 Roon Bridge / RAATServer 在用，正常现象
```

这是自动检测，无需处理。如果是你自己的程序占了必要端口，请关掉再重启 Beacon。

### 手机能发现但进不去

看 TCP 转发日志里的下行 / 上行流量：

```
[TCP:9330] 断开 192.168.1.20:54212 ↓156321B ↑2340B
```

下行几十 KB 以上说明在正常拉音乐库数据。如果下行很小或立即断开，可能是远端 Core 拒绝了（检查 Core 授权、同网段白名单设置等）。

### 能发现但 Remote 进不去 / 黑屏 / 闪退

常见于以下情况：
1. **SOOD 响应字段不够**：Remote 校验 Core 身份时字段对不上。用 `RUST_LOG=debug` 启动，看 `Core 字段:` 日志确认从 Core 探测到的字段完整。
2. **unique_id 不稳定**：Remote 把每次"新"的 Core 当作陌生设备。本程序已经用 Core 真实的 unique_id（不改写），一般不会遇到这个问题。
3. **多个桥接同时在跑**：端口被抢，部分转发没起来。`pkill -9 roon-beacon` 清理后只启一个。
4. **端口 TIME_WAIT 堆积**：频繁重启桥接会导致 TCP 端口 TIME_WAIT。等 30-60 秒或直接 `sudo ss -tln | grep 9330` 确认端口可用再重启。

### 播放卡顿 / "太慢自动跳过" / Hi-Res 不稳定

这不是桥接能单独解决的 —— 本机 RAATServer 是**直接连远端 Core 的**（不走桥接），走的是原生 VPN / Tailscale 链路。带宽不足或链路乱序会导致卡顿。诊断步骤：

```bash
# 看 RAATServer ↔ Core 连接质量
ss -tni 'dst <Core的IP>'
```

关注这几个字段：
- `rcv_ooopack`：乱序包数量，几十以上说明路径质量差
- `retrans`：重传数量
- `cwnd`：拥塞窗口，被压到 10（最小值）说明链路受限
- `send`：实际带宽

FLAC 需要的持续带宽：
- 16/44.1：~900 Kbps
- 24/96：~4 Mbps
- 24/192：~9 Mbps
- DSD64：~5.6 Mbps

**优化方案（按效果排序）：**

1. **做端口映射绕过 VPN**（最有效）：在路由器上把 TCP 9100/9101/9150/9330/9331/9332/55000 映射到 NAS，桥接 `-r` 用公网地址。
2. **启用 BBR 拥塞控制**：对 VPN 链路的乱序不敏感。
   ```bash
   sudo modprobe tcp_bbr
   sudo sysctl -w net.core.default_qdisc=fq
   sudo sysctl -w net.ipv4.tcp_congestion_control=bbr
   ```
3. **调整 Tailscale MTU**：默认 1280，可以试 1360。
   ```bash
   sudo ip link set tailscale0 mtu 1360
   ```
4. **让 Core 转码**：Roon Core 设置 → 音频 → 为这个 zone 做品质转换（降到 16/44.1）。

## 排查笔记

记录一次实际搭建中遇到的问题、原因、修法。如果你的症状类似可以直接对号入座。

### 问题 1：Remote 发现到设备但点击后进不去

**现象**：手机 Roon Remote 能看到 Core 名字（"Roon Core (Beacon)"），点进去卡住或转圈。

**根因**：SOOD 响应里的 `display_name / unique_id / service_id` 必须和真实 Core 一致。早期版本把这些字段覆盖成了桥接自己的值（`Roon Core (Beacon)` / `roon-beacon-192-168-1-10`），Remote 连上后发现 Core 身份对不上，拒绝建立会话。

**修法**：启动时先向 Core 发 SOOD 探测，用 Core 真实字段作模板；手机查询时实时透传给 Core，把 Core 真包原样回给手机。代码里 `make_sood_response_from_template` 只改 `_tid` 和 IP 字段，其他字段全部保留。

### 问题 2：同一个查询 tid 收到两次，Remote 忽略响应

**现象**：日志里看到相同 tid 被处理 2 次，Remote 表现为"时好时坏"。

**根因**：手机 WiFi 接口 + 移动数据接口都会发同一个 SOOD 查询，或者 Roon app 自己重试。如果两次响应内容不一致（一次真包一次模板），Remote 会丢弃。

**修法**：加 60s tid 缓存，**只缓存 Core 真包响应**，模板响应不缓存。同 tid 复用同一份真包字节。

### 问题 3：端口被本机 RAATServer 占用导致桥接启动失败

**现象**：启动日志里大量 `跳过 TCP :9XXX`，桥接 TCP 转发端口没起来。

**根因**：同机已经装了 Roon Bridge / RAATServer（`mono-sgen` 进程），它自己占了 9200 等端口。加上桥接频繁重启后 TIME_WAIT 堆积。

**修法**：
- 9200 被 RAATServer 占是正常的，桥接自动跳过
- 不是 RAATServer 端口的冲突：`sudo ss -tln | grep <端口>` 找进程，或等 60s TIME_WAIT 释放
- `--skip-ports 9200,...` 可手动指定跳过

### 问题 4：手机搜索不到 Core（完全没 SOOD 查询过来）

**现象**：桥接日志里只有自己主动广播 `📡 SOOD 多播`，**0 条** `🔍 SOOD 查询 from 192.168.x.x`。

**根因**：
- 手机 Roon app 用了本地缓存的 Core 连接信息，没在扫描
- 或者路由器屏蔽了 IGMP 多播

**修法**：
- 清手机 Roon Remote 缓存后重开
- 或加 `--use-broadcast` 改用 UDP 广播代替多播

### 问题 5：Tailscale 链路乱序导致播放卡顿

**现象**：控制正常、能进 Core、能播放，但放一会儿就"太慢自动播放下一首"，音频明显卡顿。

**诊断**：
```bash
ss -tni 'dst 100.107.146.8'
# 看到 rcv_ooopack:575，cwnd:10（被压制），send 只有 8-13 Mbps
```

**根因**：Tailscale 的 UDP WireGuard 通道在某些路径下乱序严重（`rcv_ooopack` 几百）。Linux 默认 TCP 拥塞控制 `cubic` 把乱序当作丢包，疯狂降速，拥塞窗口 `cwnd` 被压到最小值 10。实际带宽跑不上去，Hi-Res FLAC 需要的持续带宽达不到，Roon 判定超时自动跳过。

**修法（按有效性）**：

a. **启用 BBR**（显著但不完全解决）：
```bash
sudo modprobe tcp_bbr
sudo sysctl -w net.core.default_qdisc=fq
sudo sysctl -w net.ipv4.tcp_congestion_control=bbr
sudo sysctl -w net.core.rmem_max=16777216
sudo sysctl -w net.core.wmem_max=16777216
sudo sysctl -w net.ipv4.tcp_rmem="4096 131072 16777216"
sudo sysctl -w net.ipv4.tcp_wmem="4096 65536 16777216"
sudo sysctl -w net.ipv4.tcp_reordering=6
sudo ip link set tailscale0 mtu 1360
# 重启 RAATServer 让新连接用新参数
sudo systemctl restart roonbridge.service
```

持久化把上面参数（除 modprobe / ip link）写到 `/etc/sysctl.d/99-roon-tailscale.conf`。

b. **在 NAS 的上游路由器做端口映射**（彻底解决）：把 TCP 9100/9101/9150/9330/9331/9332/55000 映射到 NAS，桥接 `-r` 用公网地址。这样音频流走公网 BBR 链路，不走 Tailscale。

c. **Roon Core 转码降采样**：设置 → 音频 → 当前 zone → 品质转换 → 16/44.1。应急用。

### 问题 6：用 Tailscale 主机名（或自定义 DNS 域名）启动解析到 IPv6 不通

**现象**：
```
域名解析: lazycat → fc03:1136:...:0
无法连接到 [fc03:...]:9003: Address family not supported
```

**根因**：一些内网 DNS（例如懒猫 SDN）只返回私有 IPv6 地址，而本机 UDP socket 代码只建 IPv4。

**修法**：`--prefer v4` 强制用 IPv4 解析；如果域名没 IPv4 记录，换用 Tailscale IP（`tailscale ip -4 <机器名>` 查）。

### 问题 7：反向代理开启后所有设备变成"电话的音频"

**现象**：`--reverse-proxy` 模式下，本机 RAAT Bridge 的所有设备在 Core 里都被归类到"This Device"。

**根因**：反向代理把广告 IP 改写成桥接 IP，Core 看到 Remote 和 Output 的源 IP 都是同一个（桥接 IP），误判为"同一台设备"。

**修法**：除非确实需要 Core 发现本地 RAAT 设备，否则**不要开 `--reverse-proxy`**。

### 问题 8：RAATServer 还连着 Tailscale IP，没切到桥接

**现象**：桥接启动后，`ss -tn` 显示 RAATServer 的 TCP 连接还指向 Core 的 Tailscale IP（100.x.x.x），没连桥接的本地 IP。

**根因**：RAATServer 已有活跃会话，不会主动断开重连。SOOD 只在启动时扫描一次。

**修法**：
```bash
sudo systemctl restart roonbridge.service
```

重启后 RAATServer 重新扫描，收到桥接广告后切到本地 IP。

## 关于手机自身出声

安卓 Roon Remote **只是 Control 角色，不是 Output**。即使 Core 发现了手机，也不会把手机音频作为可选设备。要手机出声只有两个方案：

- **Roon ARC**（官方订阅服务）
- 在手机上跑一个能充当 RAAT Bridge 的第三方 app（Roon 没提供官方 SDK）

Roon Beacon 只负责 Core 发现和转发，没法改变 Roon 的这个限制。

## 关于反向 SOOD 代理

`--reverse-proxy` 让本地的 RAAT Bridge 设备能被远端 Core 发现。但有一个副作用：Core 看到 **Remote 源 IP 和 Output 源 IP 都是桥接 IP**，可能把所有本地 Output 归类到"This Device"（电话的音频）下。

一般情况下**不建议开启**。只有当你确实有一台本地 RAAT 设备，而且希望 Core 能直接通过桥接找到它，才需要打开。

## 许可

MIT
