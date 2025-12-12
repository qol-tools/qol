#!/bin/bash
ACTION_ID="$1"

case "$ACTION_ID" in
    status)
        if curl -s http://localhost:42710/health > /dev/null 2>&1; then
            osascript -e 'display notification "Task Runner daemon is running on port 42710" with title "Task Runner"' 2>/dev/null || \
            notify-send "Task Runner" "Daemon is running on port 42710" 2>/dev/null || \
            echo "Task Runner daemon is running on port 42710"
        else
            osascript -e 'display notification "Task Runner daemon is NOT running" with title "Task Runner"' 2>/dev/null || \
            notify-send "Task Runner" "Daemon is NOT running" 2>/dev/null || \
            echo "Task Runner daemon is NOT running"
        fi
        ;;
    *)
        echo "Unknown action: $ACTION_ID"
        ;;
esac
