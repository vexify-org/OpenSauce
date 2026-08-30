#!/usr/bin/env python3
import os, pty, sys, time, select, fcntl, termios, struct
import pyte

cols = int(os.environ.get("COLS", 100))
rows = int(os.environ.get("ROWS", 30))

screen = pyte.Screen(cols, rows)
stream = pyte.Stream(screen)

pid, fd = pty.fork()
if pid == 0:
    os.environ["TERM"] = "xterm-256color"
    os.execv("/workspace/target/debug/opensauce", ["opensauce"])
    os._exit(0)

# set window size on the slave / controlling tty
fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))

def read_avail(seconds):
    end = time.time() + seconds
    buf = b""
    while time.time() < end:
        r, _, _ = select.select([fd], [], [], 0.05)
        if fd in r:
            try:
                d = os.read(fd, 4096)
            except OSError:
                break
            if not d:
                break
            buf += d
        else:
            break
    return buf

def drain(seconds):
    buf = read_avail(seconds)
    if buf:
        stream.feed(buf.decode("utf-8", "replace"))

time.sleep(0.8)
drain(0.6)

for ch in "hello opencode":
    os.write(fd, ch.encode())
    time.sleep(0.015)
os.write(fd, b"\r")
time.sleep(0.6)
drain(0.6)
time.sleep(0.8)
drain(1.0)
time.sleep(1.0)
drain(1.0)
time.sleep(1.0)
drain(1.0)

try:
    os.kill(pid, 15)
except Exception:
    pass

for y in range(rows):
    print("".join((screen.buffer[y][x].data or " ") for x in range(cols)).rstrip() or "·")
print("===== END =====")