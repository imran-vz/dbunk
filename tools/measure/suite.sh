#!/bin/sh
# Plan 024: the same measurement sequence for either desktop host.
#
#   suite.sh <pid> <out-dir> <grid-point fx,fy>
#
# Expects the app running with the shared 2,000-line document open and the
# editor focused. Every key and scroll event is posted to <pid> only. The
# window is brought to the front and has to stay there for the whole run
# (about four minutes): do not use the machine meanwhile.
set -eu
PID=$1
OUT=$2
GRID=$3
M="$(dirname "$0")/.build/release/measure"
RUNS=${RUNS:-3}
KEYS=${KEYS:-300}
SECTIONS=${SECTIONS:-typing idle fixtures}
# Scroll steps are posted at twice the display's refresh rate. At or below
# the refresh rate some refresh intervals get no step, the app has nothing new
# to draw in them, and they would be counted as long frames.
SCROLL_HZ=${SCROLL_HZ:-240}
wants() { case " $SECTIONS " in *" $1 "*) return 0 ;; *) return 1 ;; esac; }
UP=126 DOWN=125 RIGHT=124 RETURN=36
mkdir -p "$OUT"

top_of_document() { "$M" chord --pid "$PID" --key $UP --command; sleep 0.3; }

if wants typing; then
  echo "== typing: $RUNS runs of $KEYS keys at the end of line 1 (a comment)"
  top_of_document
  "$M" chord --pid "$PID" --key $RIGHT --command
  sleep 0.5
  for run in $(seq 1 "$RUNS"); do
    "$M" latency --pid "$PID" --count "$KEYS" --interval-ms 120 --out "$OUT/latency-$run.json"
  done
fi

if wants idle; then
  echo "== idle: 30 s with nothing happening but the caret"
  sleep 2
  "$M" footprint --pid "$PID" --seconds 30 --out "$OUT/idle.json"
fi

wants fixtures || { echo "== done: $OUT"; exit 0; }

line=1
for fixture in wide large many; do
  line=$((line + 1))
  echo "== $fixture: run the statement on line $line, then scroll"
  top_of_document
  for _ in $(seq 2 "$line"); do "$M" chord --pid "$PID" --key $DOWN; sleep 0.1; done
  "$M" chord --pid "$PID" --key $RETURN --command
  sleep 8
  "$M" footprint --pid "$PID" --seconds 5 --out "$OUT/footprint-$fixture.json"
  # About 4,000 points a second down the long fixtures, 1,000 down the
  # 400-row one, 2,000 sideways.
  delta=-20
  [ "$fixture" = large ] && delta=-5
  for run in $(seq 1 "$RUNS"); do
    "$M" scroll --pid "$PID" --at "$GRID" --seconds 5 --delta "$delta" --hz "$SCROLL_HZ" --out "$OUT/scroll-$fixture-down-$run.json"
    # Back to the top for the next run, unmeasured.
    "$M" scroll --pid "$PID" --at "$GRID" --seconds 3 --delta 2000 --out /dev/null
  done
  if [ "$fixture" = wide ]; then
    for run in $(seq 1 "$RUNS"); do
      "$M" scroll --pid "$PID" --at "$GRID" --seconds 4 --delta -10 --hz "$SCROLL_HZ" --horizontal --out "$OUT/scroll-wide-right-$run.json"
      "$M" scroll --pid "$PID" --at "$GRID" --seconds 3 --delta 2000 --horizontal --out /dev/null
    done
  fi
done
echo "== done: $OUT"
