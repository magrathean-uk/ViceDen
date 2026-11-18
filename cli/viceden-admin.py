#!/usr/bin/env python3
"""
Admin CLI for VICEDEN Forum
=======================

This script provides administrative functionality for the VICEDEN forum.
It is intentionally minimal and mostly serves as a reference for an
follow-up work to implement the full set of admin commands.  The goal is
to perform CRUD operations on users, threads and posts without
embedding SQL in the CLI; all operations should go through the
server's JSON API or a dedicated admin API.  You may also connect
directly to the database using SQLx or psycopg2 if appropriate.

Usage examples:

```bash
./viceden-admin.py create-user --handle alice --password secret
./viceden-admin.py list-users
./viceden-admin.py ban-user --handle bob
./viceden-admin.py delete-thread --id 123e4567-e89b-12d3-a456-426614174000
```

Currently the script does not implement any real API calls; it
demonstrates the argument parsing structure and outlines where
functionality should be added.  See `docs/reference/admin-cli-spec.md` for
detailed requirements.
"""

import argparse
import sys
from dataclasses import dataclass
from typing import Optional

# Data classes representing potential admin commands.  In a full
# implementation these would be converted into API requests or SQL
# queries.  For now they just print debug information.

@dataclass
class CreateUser:
    handle: str
    password: str

    def run(self):
        # TODO: call /api/admin/users POST
        print(f"[ADMIN] Creating user: handle={self.handle}")

@dataclass
class ListUsers:
    def run(self):
        # TODO: call /api/admin/users GET
        print("[ADMIN] Listing users (not implemented)")

@dataclass
class BanUser:
    handle: str
    reason: Optional[str]

    def run(self):
        # TODO: call /api/admin/users/{handle}/ban POST
        print(f"[ADMIN] Banning user: handle={self.handle}, reason={self.reason}")

@dataclass
class DeleteThread:
    id: str

    def run(self):
        # TODO: call /api/admin/threads/{id} DELETE
        print(f"[ADMIN] Deleting thread id={self.id}")

def parse_args(argv):
    parser = argparse.ArgumentParser(description="VICEDEN admin CLI")
    subparsers = parser.add_subparsers(dest="command", required=True)

    # create-user
    p_create = subparsers.add_parser("create-user", help="Create a new user")
    p_create.add_argument("--handle", required=True, help="User handle (lowercase)")
    p_create.add_argument("--password", required=True, help="Password (plain text)")

    # list-users
    subparsers.add_parser("list-users", help="List all users")

    # ban-user
    p_ban = subparsers.add_parser("ban-user", help="Ban a user")
    p_ban.add_argument("--handle", required=True, help="Handle of the user to ban")
    p_ban.add_argument("--reason", help="Reason for the ban")

    # delete-thread
    p_del_thread = subparsers.add_parser("delete-thread", help="Delete a thread by ID")
    p_del_thread.add_argument("--id", required=True, help="UUID of the thread to delete")

    return parser.parse_args(argv)

def main(argv=None):
    args = parse_args(argv or sys.argv[1:])
    cmd: object
    if args.command == "create-user":
        cmd = CreateUser(handle=args.handle, password=args.password)
    elif args.command == "list-users":
        cmd = ListUsers()
    elif args.command == "ban-user":
        cmd = BanUser(handle=args.handle, reason=args.reason)
    elif args.command == "delete-thread":
        cmd = DeleteThread(id=args.id)
    else:
        raise AssertionError("Unhandled command: " + args.command)
    cmd.run()

if __name__ == "__main__":
    main()
