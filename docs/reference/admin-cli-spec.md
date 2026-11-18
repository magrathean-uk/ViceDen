# CLI status

## Administrative CLI

`cli/viceden-admin.py` is an argument-parsing prototype. Its commands print messages; they do not call the API, write the database, or administer a running forum.

| Parsed command | Accepted arguments | Current effect |
| --- | --- | --- |
| `create-user` | `--handle`, `--password` | Prints the handle |
| `list-users` | None | Prints an unimplemented message |
| `ban-user` | `--handle`, optional `--reason` | Prints the request |
| `delete-thread` | `--id` | Prints the identifier |

Use the founder web interface or documented founder API routes for actual administration. Do not pass real passwords to prototype commands or infer success from their output.

Future CLI work should use authenticated server routes, keep authorization on the server, report errors through exit codes, and test each command against the implemented API. The old speculative command list is not an API contract.

## User CLI

`cli/viceden.py` supports Python 3.10 and newer and uses the pinned `requests` dependency in `cli/requirements.txt`. Install it into a virtual environment inside the checkout with `python3 -m venv .venv` and `.venv/bin/python -m pip install -r cli/requirements.txt`, not into a system or user Python, and run the script with `.venv/bin/python`. Its base URL is hard-coded to `http://localhost:3000/api`. The `--token` argument supplies a PAT, and `cli/viceden.sh` forwards arguments to Python.

The parser exposes `boards`, `threads`, `thread-view`, `thread-create`, and `reply`. The implementation makes HTTP requests for all five, including writes; its opening comment claiming writes are stubbed is stale. The server requires a PAT for these operations. Passing tokens on a command line can expose them in shell history or process listings.

No Python dependency lockfile or packaged CLI distribution is provided. This script is a lightweight reference, not a separately supported client release. See the [API reference](api-v1-spec.md) for routes and payloads.
