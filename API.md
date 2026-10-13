# wallctl local API

Version 1 is a line-oriented protocol over the daemon's Unix domain socket:
`~/.config/wallctl/daemon.sock`. Send one UTF-8 request, shut down the write
side of the connection, and read the JSON response until the server closes its
side. The socket is private to the current user. Indexes are one-based and are
valid for the active playlist order until it is reloaded or the daemon restarts.

| Request | Purpose |
| --- | --- |
| `API/1 STATUS` | Read the active profile, pause state, and current wallpaper. |
| `API/1 LIST` | Get playlist entries, indexes, paths, and current selection. |
| `API/1 SET <index>` | Select a playlist entry. |
| `API/1 NEXT` / `API/1 PREV` | Change the selected entry. |
| `API/1 PAUSE` / `API/1 RESUME` | Pause or resume timed rotation. |
| `API/1 SWITCH <profile>` / `API/1 RELOAD` | Change profiles or reload configuration. |
| `API/1 STOP` | Stop the daemon and restore the previous wallpaper when configured. |

Successful actions return `{"api_version":1,"ok":true}` (with additional
fields where useful). Errors return `{"api_version":1,"ok":false,"error":"..."}`.
`LIST` returns an object with `api_version` and an `items` array. Every item
has `index`, `name`, `path`, and `current` fields. `STATUS` returns
`active_profile`, `paused`, `total_items`, `current_index`, and `current_path`.

Example Python client using only the standard library:

```python
import json
import socket
from pathlib import Path

socket_path = Path.home() / ".config/wallctl/daemon.sock"

def request(command):
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as client:
        client.connect(str(socket_path))
        client.sendall(("API/1 " + command).encode())
        client.shutdown(socket.SHUT_WR)
        chunks = []
        while chunk := client.recv(65536):
            chunks.append(chunk)
    return json.loads(b"".join(chunks))

playlist = request("LIST")
for item in playlist["items"]:
    print(item["index"], item["name"])

request("SET 2")
```

The CLI `wallctl list` and `wallctl set <index>` use the same active playlist
and index convention. The API is local to the logged-in user session; it is not
an HTTP service and does not listen on the network.
