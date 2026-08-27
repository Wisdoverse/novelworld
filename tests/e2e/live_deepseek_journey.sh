#!/usr/bin/env bash
set -euo pipefail

python_bin=${PYTHON:-python3}
exec "$python_bin" "$(dirname "$0")/live_deepseek_journey.py" "$@"
