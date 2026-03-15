#!/bin/bash
echo '{"action":"toggle-main"}' | socat - UNIX-CONNECT:/tmp/plugin-lights-debug.sock
