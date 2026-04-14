#!/bin/sh
set -e

# Initialize Gemini CLI configuration directory and file if they don't exist.
# This prevents ENOENT/SyntaxError during first-time startup.
mkdir -p /home/node/.gemini

if [ ! -f "/home/node/.gemini/projects.json" ]; then
    echo "{}" > /home/node/.gemini/projects.json
fi

exec "$@"
