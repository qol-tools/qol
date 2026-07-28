#!/usr/bin/env bash
set -euo pipefail

gdbus call \
  --session \
  --dest org.Cinnamon \
  --object-path /org/qol/AltTabPreviewPlane \
  --method org.qol.AltTabPreviewPlane.ShowDemo
