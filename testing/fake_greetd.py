#!/usr/bin/env python3
"""A stand-in greetd: real socket, real framing, scripted replies.

For driving the greeter without installing greetd and without going
anywhere near how this machine logs in. It speaks the actual wire format
— native-endian u32 length, then JSON — so a mistake in the framing shows
up here rather than at a login prompt.

    ./fake_greetd.py sock [password]
    cd <that dir> && GREETD_SOCK=sock hyprforge-greet --user you --type-in <password>

`GREETD_SOCK` has to be a short relative path: AF_UNIX caps paths at
about 108 bytes, which a scratchpad directory can exceed. Real greetd
uses /run/greetd.sock, so this constraint is the test harness's alone.
"""
import json, os, pathlib, socket, struct, sys

# AF_UNIX paths cap at ~108 bytes, which the scratchpad exceeds, so bind
# a relative name from inside the directory instead of an absolute one.
target = pathlib.Path(sys.argv[1]).resolve()
os.chdir(target.parent)
path = target.name
password = sys.argv[2] if len(sys.argv) > 2 else "hunter2"
if os.path.exists(path):
    os.unlink(path)
srv = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
srv.bind(path); srv.listen(1)
print(f"fake greetd listening on {path}", flush=True)

def read(conn):
    head = conn.recv(4)
    if len(head) < 4:
        return None
    (n,) = struct.unpack("=I", head)
    return json.loads(conn.recv(n))

def write(conn, obj):
    body = json.dumps(obj).encode()
    conn.sendall(struct.pack("=I", len(body)) + body)

conn, _ = srv.accept()
while True:
    req = read(conn)
    if req is None:
        break
    print("  <-", req, flush=True)
    t = req["type"]
    if t == "create_session":
        write(conn, {"type": "auth_message", "auth_message_type": "secret",
                     "auth_message": "Password:"})
    elif t == "post_auth_message_response":
        if req.get("response") == password:
            write(conn, {"type": "success"})
        else:
            write(conn, {"type": "error", "error_type": "auth_error",
                         "description": "Authentication failure"})
    elif t == "start_session":
        print("  == session scheduled:", req["cmd"], flush=True)
        write(conn, {"type": "success"})
    else:
        write(conn, {"type": "error", "error_type": "error",
                     "description": f"unexpected {t}"})
