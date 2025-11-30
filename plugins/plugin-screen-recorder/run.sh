#!/usr/bin/env bash

# deps: slop, ffmpeg, xrandr, jq
# QoL Tray plugin for screen recording with audio support

set -euo pipefail

# ============================================================================
# CONFIGURATION
# ============================================================================

# Get the directory where this script is located
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CONFIG_FILE="$SCRIPT_DIR/config.json"

pidfile="/tmp/record-region.pid"
logfile="/tmp/record-region.log"
indicator_pidfile="/tmp/record-region-indicator.pid"
color="${1:-#ff2b2b}"
thickness="${2:-4}"
snap_margin_px=50  # snap to screen edge if within this many pixels

# Load config from JSON
load_config() {
  if [[ ! -f "$CONFIG_FILE" ]]; then
    echo "Warning: config.json not found, using defaults" >&2
    return 1
  fi

  # Parse config using jq
  AUDIO_ENABLED=$(jq -r '.audio.enabled // true' "$CONFIG_FILE")
  AUDIO_MIC_DEVICE=$(jq -r '.audio.mic_device // "default"' "$CONFIG_FILE")
  AUDIO_SYSTEM_DEVICE=$(jq -r '.audio.system_device // "default"' "$CONFIG_FILE")
  VIDEO_CRF=$(jq -r '.video.crf // 18' "$CONFIG_FILE")
  VIDEO_PRESET=$(jq -r '.video.preset // "veryfast"' "$CONFIG_FILE")
  VIDEO_FRAMERATE=$(jq -r '.video.framerate // 60' "$CONFIG_FILE")
  VIDEO_FORMAT=$(jq -r '.video.format // "mkv"' "$CONFIG_FILE")

  # Parse audio inputs array
  AUDIO_INPUTS=$(jq -r '.audio.inputs // ["mic"] | join(",")' "$CONFIG_FILE")
}

# Load config at startup
load_config || true

# ============================================================================
# HELPER FUNCTIONS
# ============================================================================

show_notification() {
  local title="$1"
  local message="$2"
  local timeout="${3:-800}"
  notify-send -u normal -t "$timeout" "$title" "$message"
}

show_recording_indicator() {
  # no indicator - just notifications
  :
}

hide_recording_indicator() {
  # no indicator to hide
  :
}

get_monitor_for_selection() {
  local sel_x=$1 sel_y=$2 sel_w=$3 sel_h=$4
  local center_x=$(( sel_x + sel_w / 2 ))
  local center_y=$(( sel_y + sel_h / 2 ))

  while IFS= read -r line; do
    if [[ "$line" =~ ^([A-Z0-9-]+)\ connected\ (primary\ )?([0-9]+)x([0-9]+)\+(-?[0-9]+)\+(-?[0-9]+) ]]; then
      local mon_name="${BASH_REMATCH[1]}"
      local mon_w="${BASH_REMATCH[3]}"
      local mon_h="${BASH_REMATCH[4]}"
      local mon_x="${BASH_REMATCH[5]}"
      local mon_y="${BASH_REMATCH[6]}"

      # check if selection center is within this monitor
      if (( center_x >= mon_x && center_x < mon_x + mon_w &&
            center_y >= mon_y && center_y < mon_y + mon_h )); then
        echo "$mon_name $mon_w $mon_h $mon_x $mon_y"
        return 0
      fi
    fi
  done < <(xrandr --query)

  return 1
}

clamp_to_bounds() {
  local bounds_x=$1 bounds_y=$2 bounds_w=$3 bounds_h=$4

  # clamp x,y to bounds
  if (( x < bounds_x )); then
    w=$(( w - (bounds_x - x) ))
    x=$bounds_x
  fi
  if (( y < bounds_y )); then
    h=$(( h - (bounds_y - y) ))
    y=$bounds_y
  fi

  # clamp width/height to bounds
  if (( x + w > bounds_x + bounds_w )); then
    w=$(( bounds_x + bounds_w - x ))
  fi
  if (( y + h > bounds_y + bounds_h )); then
    h=$(( bounds_y + bounds_h - y ))
  fi
}

# ============================================================================
# MAIN LOGIC
# ============================================================================

# Check if already recording - if so, stop it
if [[ -f "$pidfile" ]]; then
  pid=$(<"$pidfile")
  if kill -0 "$pid" 2>/dev/null; then
    kill -INT "$pid"
    wait "$pid" 2>/dev/null || true
    rm -f "$pidfile"
    hide_recording_indicator

    # show notification with clickable action to open Videos folder
    notify-send -u normal -t 5000 "⏹️  Recording stopped" "Saved to ~/Videos" \
      --action="default=Open Folder" | while read action; do
        if [[ "$action" == "default" ]]; then
          xdg-open "$HOME/Videos" &
        fi
      done &

    exit 0
  else
    rm -f "$pidfile"
  fi
fi

# Get user selection
geom=$(slop --highlight --color=1,0,0,0.65 -b 0 -f '%x,%y,%w,%h') || geom=""
[[ -z "$geom" ]] && exit 0

# Parse selection geometry
IFS=',' read -r x y w h <<<"$geom"
echo "Original selection: ${x},${y} ${w}x${h}"

# Detect which monitor this selection is on
if monitor_info=$(get_monitor_for_selection "$x" "$y" "$w" "$h"); then
  read -r mon_name mon_w mon_h mon_x mon_y <<<"$monitor_info"
  echo "Selection is on monitor: $mon_name (${mon_w}x${mon_h}+${mon_x}+${mon_y})"

  # Clamp to monitor bounds
  clamp_to_bounds "$mon_x" "$mon_y" "$mon_w" "$mon_h"
  screen_bottom=$(( mon_y + mon_h ))
else
  # Fallback: clamp to full virtual screen
  echo "Could not detect monitor, using full screen bounds"
  read -r screen_w screen_h < <(xdpyinfo | awk '/dimensions:/{split($2,a,"x"); print a[1],a[2]; exit}')
  clamp_to_bounds 0 0 "$screen_w" "$screen_h"
  screen_bottom=$screen_h
fi

echo "After clamping: ${x},${y} ${w}x${h}"

# Snap to bottom edge if close enough (for capturing taskbar)
if [[ -n "${screen_bottom:-}" ]]; then
  gap_to_bottom=$(( screen_bottom - (y + h) ))
  if (( gap_to_bottom > 0 && gap_to_bottom <= snap_margin_px )); then
    echo "Snapping to bottom edge (gap was ${gap_to_bottom}px)"
    h=$(( screen_bottom - y ))
  fi
fi

# Validate dimensions
if (( w <= 0 || h <= 0 )); then
  show_notification "❌ Recording failed" "Invalid area: ${w}x${h}"
  exit 1
fi

# Ensure even dimensions (required by x264)
(( w % 2 != 0 )) && ((w--))
(( h % 2 != 0 )) && ((h--))

echo "Final recording area: ${x},${y} ${w}x${h}"

# Build ffmpeg command with optional audio
outfile="$HOME/Videos/recording-$(date +%F_%H-%M-%S).${VIDEO_FORMAT:-mkv}"

# Base ffmpeg command
ffmpeg_cmd=(
  ffmpeg
  -f x11grab
  -video_size "${w}x${h}"
  -framerate "${VIDEO_FRAMERATE:-60}"
  -i ":0.0+${x},${y}"
)

# Add audio inputs if enabled
if [[ "${AUDIO_ENABLED:-false}" == "true" ]]; then
  echo "Audio enabled: $AUDIO_INPUTS"

  has_mic=false
  has_system=false

  if [[ "$AUDIO_INPUTS" == *"mic"* ]]; then
    has_mic=true
  fi

  if [[ "$AUDIO_INPUTS" == *"system"* ]]; then
    has_system=true
  fi

  if $has_mic && $has_system; then
    # Both mic and system audio - mix them together
    ffmpeg_cmd+=(
      -f pulse -i "$AUDIO_MIC_DEVICE"
      -f pulse -i "${AUDIO_SYSTEM_DEVICE}.monitor"
      -filter_complex "[1:a][2:a]amerge=inputs=2[aout]"
      -map 0:v -map "[aout]"
      -c:a aac -b:a 192k
    )
  elif $has_mic; then
    # Mic only
    ffmpeg_cmd+=(
      -f pulse -i "$AUDIO_MIC_DEVICE"
      -c:a aac -b:a 192k
    )
  elif $has_system; then
    # System audio only
    ffmpeg_cmd+=(
      -f pulse -i "${AUDIO_SYSTEM_DEVICE}.monitor"
      -c:a aac -b:a 192k
    )
  fi
else
  echo "Audio disabled"
fi

# Add video encoding options
ffmpeg_cmd+=(
  -c:v libx264
  -crf "${VIDEO_CRF:-18}"
  -preset "${VIDEO_PRESET:-veryfast}"
  -pix_fmt yuv420p
  "$outfile"
)

# Execute ffmpeg
echo "Running: ${ffmpeg_cmd[*]}"
"${ffmpeg_cmd[@]}" </dev/null &>"$logfile" &

ffmpeg_pid=$!
echo $ffmpeg_pid > "$pidfile"

# Verify ffmpeg started successfully
sleep 0.5
if kill -0 "$ffmpeg_pid" 2>/dev/null; then
  show_notification "🔴 Recording started" "Press your hotkey to stop"
  show_recording_indicator
  disown
else
  rm -f "$pidfile"
  show_notification "❌ Recording failed" "Check $logfile"
  exit 1
fi
