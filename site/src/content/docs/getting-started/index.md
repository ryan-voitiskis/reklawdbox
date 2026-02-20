---
title: Getting Started
description: Set up reklawdbox and connect it to your MCP host.
---

## Summary

This guide gets a local reklawdbox binary running as an MCP server over stdio.

## Steps

1. Build the binary:

   ```bash
   cargo build --release
   ```

2. Create local MCP config from the example:

   ```bash
   cp mcp-config.example.json .mcp.json
   ```

3. Edit `.mcp.json` and set required environment values (for example `REKORDBOX_DB_PATH` if you do not use the default path).
4. Register the command `./target/release/reklawdbox` with your MCP host using `stdio` transport.
5. Run a smoke call such as `read_library` to verify connectivity.

## Examples

Minimal command-only startup flow:

```bash
cargo build --release
cp mcp-config.example.json .mcp.json
```

## Related

- [Concepts](/concepts/)
- [Reference](/reference/)
- [Troubleshooting](/troubleshooting/)
