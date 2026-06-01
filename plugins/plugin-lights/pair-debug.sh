#!/bin/bash
echo '{"action":"pair"}' | socat - UNIX-CONNECT:/tmp/plugin-lights-debug.sock
