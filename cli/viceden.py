#!/usr/bin/env python3
"""
User CLI for VICEDEN Forum
======================

This script allows command‑line interaction with the VICEDEN forum's
public JSON API.  It is deliberately lightweight and currently
provides only read operations; posting threads/replies is stubbed.
Users authenticate by passing an API token via the `--token`
argument.  In the future you may support username/password login and
session management.

Example usage:

```bash
./viceden.py boards
./viceden.py threads general
./viceden.py thread-view 123e4567-e89b-12d3-a456-426614174000
```

See `docs/reference/api-v1-spec.md` for endpoint descriptions.
"""

import argparse
import json
import sys
import requests
from typing import Optional

API_BASE = "http://localhost:3000/api"

def list_boards(token: Optional[str]) -> None:
    url = f"{API_BASE}/boards"
    headers = {}
    if token:
        headers["Authorization"] = f"Bearer {token}"
    resp = requests.get(url, headers=headers)
    resp.raise_for_status()
    data = resp.json()
    for board in data.get("boards", []):
        print(f"{board['slug']}: {board['name']} (TTL {board['ttl_days']} days)")

def list_threads(slug: str, token: Optional[str]) -> None:
    url = f"{API_BASE}/boards/{slug}/threads"
    headers = {}
    if token:
        headers["Authorization"] = f"Bearer {token}"
    resp = requests.get(url, headers=headers)
    if resp.status_code == 501:
        print("Listing threads is not implemented yet.")
        return
    resp.raise_for_status()
    threads = resp.json()
    for thread in threads:
        print(f"{thread['id']}: {thread['title']}")

def view_thread(thread_id: str, token: Optional[str]) -> None:
    url = f"{API_BASE}/threads/{thread_id}/posts"
    headers = {}
    if token:
        headers["Authorization"] = f"Bearer {token}"
    resp = requests.get(url, headers=headers)
    if resp.status_code == 501:
        print("Viewing posts is not implemented yet.")
        return
    resp.raise_for_status()
    posts = resp.json()
    for idx, post in enumerate(posts, start=1):
        print(f"#{idx} ({post['created_at']}): {post['body']}")

def create_thread(slug: str, title: str, body: str, token: Optional[str]) -> None:
    url = f"{API_BASE}/boards/{slug}/threads"
    headers = {"Content-Type": "application/json"}
    if token:
        headers["Authorization"] = f"Bearer {token}"
    payload = {"title": title, "body": body}
    resp = requests.post(url, headers=headers, data=json.dumps(payload))
    if resp.status_code == 501:
        print("Thread creation is not implemented yet.")
        return
    resp.raise_for_status()
    print("Thread created:", resp.json())

def create_post(thread_id: str, body: str, token: Optional[str]) -> None:
    url = f"{API_BASE}/threads/{thread_id}/posts"
    headers = {"Content-Type": "application/json"}
    if token:
        headers["Authorization"] = f"Bearer {token}"
    payload = {"body": body}
    resp = requests.post(url, headers=headers, data=json.dumps(payload))
    if resp.status_code == 501:
        print("Replying is not implemented yet.")
        return
    resp.raise_for_status()
    print("Reply posted:", resp.json())

def parse_args(argv):
    parser = argparse.ArgumentParser(description="VICEDEN user CLI")
    parser.add_argument("command", choices=["boards", "threads", "thread-view", "thread-create", "reply"], help="Command to execute")
    parser.add_argument("param1", nargs="?", help="Board slug or thread ID depending on command")
    parser.add_argument("param2", nargs="?", help="Title or body for creation commands")
    parser.add_argument("param3", nargs="?", help="Body for thread creation when both title and body are needed")
    parser.add_argument("--token", help="API token for authentication")
    return parser.parse_args(argv)

def main(argv=None) -> None:
    args = parse_args(argv or sys.argv[1:])
    if args.command == "boards":
        list_boards(args.token)
    elif args.command == "threads":
        if not args.param1:
            print("Board slug required.")
            return
        list_threads(args.param1, args.token)
    elif args.command == "thread-view":
        if not args.param1:
            print("Thread ID required.")
            return
        view_thread(args.param1, args.token)
    elif args.command == "thread-create":
        if not args.param1 or not args.param2 or not args.param3:
            print("Usage: thread-create <board_slug> <title> <body>")
            return
        create_thread(args.param1, args.param2, args.param3, args.token)
    elif args.command == "reply":
        if not args.param1 or not args.param2:
            print("Usage: reply <thread_id> <body>")
            return
        create_post(args.param1, args.param2, args.token)
    else:
        print("Unknown command")

if __name__ == "__main__":
    main()