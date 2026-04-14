#!/bin/sh
set -e

# Initialize Gemini CLI configuration directory and file if they don't exist.
# This prevents ENOENT/SyntaxError during first-time startup.
if [ ! -d "/home/node/.gemini" ]; then
    mkdir -p /home/node/.gemini
fi

if [ ! -f "/home/node/.gemini/projects.json" ]; then
    echo "{}" > /home/node/.gemini/projects.json
fi

# Ensure correct permissions for the node user
chmod -R 777 /home/node/.gemini

exec "$@"
