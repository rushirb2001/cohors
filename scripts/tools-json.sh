#!/bin/sh
# Regenerate tools.json, the MCP tool catalog exactly as `cohors mcp` answers
# `tools/list`, for directories that list MCP servers without running them
# (mcprush reads a root tools.json; it cannot read tools out of Rust source).
# A test in cohors-mcp fails whenever the committed file and the catalog
# differ, and this writes it through that same test.
set -eu
cd "$(dirname "$0")/.."
COHORS_UPDATE_TOOLS_JSON=1 cargo test -q -p cohors-mcp the_committed_tools_json_is_the_tool_catalog >/dev/null
echo "tools.json: $(python3 -c 'import json; print(len(json.load(open("tools.json"))["tools"]))') tools"
