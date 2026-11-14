"""UDP 遥测抓包：把 vettura 遥测 JSON 帧按行写到文件。

用法: python3 scripts/capture_telemetry.py <输出文件> [端口] [超时秒]
收到超时秒内没有新帧就退出（默认 15s），保证进程不会挂着。
"""
import json
import os
import socket
import sys

OUT = sys.argv[1] if len(sys.argv) > 1 else "telemetry.jsonl"
os.makedirs(os.path.dirname(os.path.abspath(OUT)), exist_ok=True)
PORT = int(sys.argv[2]) if len(sys.argv) > 2 else 9999
IDLE_TIMEOUT = float(sys.argv[3]) if len(sys.argv) > 3 else 15.0

sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
sock.bind(("127.0.0.1", PORT))
sock.settimeout(IDLE_TIMEOUT)

count = 0
with open(OUT, "w") as f:
    while True:
        try:
            data, _ = sock.recvfrom(65536)
        except socket.timeout:
            break
        line = data.decode("utf-8", "replace")
        f.write(line + "\n")
        count += 1
        if count % 200 == 0:
            f.flush()
    f.flush()

print(f"captured {count} frames -> {OUT}")
