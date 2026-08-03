#!/usr/bin/env python3
"""
Standalone Grove Hall sensor check (no app, no network, no config.json).

Prints the *raw* GPIO level as well as the interpreted state, so a wiring or
polarity problem is visible directly instead of being hidden behind the node's
active_low logic.

    Grove Hall sensor: the output goes LOW while a magnetic field is present,
    so the line idles HIGH (via the internal pull-up) and drops when a magnet
    comes close. https://wiki.seeedstudio.com/Grove-Hall_Sensor/

Usage:
    python3 hall_test.py            # GPIO 5 (Grove Base Hat port D5)
    python3 hall_test.py 16         # another BCM pin

Move a magnet towards the sensor and away again; every stable change is
logged. Ctrl-C to stop.
"""

import sys
import time

from gpiozero import DigitalInputDevice

POLL_S = 0.02      # 50 Hz
DEBOUNCE_S = 0.03  # line must be stable this long before a change counts


def main():
    pin = int(sys.argv[1]) if len(sys.argv) > 1 else 5

    # pull_up=True gives the idle state a defined HIGH level. Note gpiozero
    # also derives the polarity from this flag (active_state = not pull_up),
    # so .value is already "triggered"; .pin.state is the raw level.
    sensor = DigitalInputDevice(pin, pull_up=True)

    print(f"Grove Hall sensor on BCM GPIO {pin} (pull-up enabled)")
    print(f"pin factory: {type(sensor.pin_factory).__name__}")
    raw = sensor.pin.state
    print(f"idle level : {'HIGH' if raw else 'LOW'}  -> "
          f"{'no magnet (expected)' if raw else 'MAGNET DETECTED'}")
    if not raw:
        print("  Reading LOW with no magnet present means the sensor is")
        print("  triggered, mis-wired, or the port is not this GPIO.")
    print("\nMove a magnet in and out. Ctrl-C to stop.\n")

    stable = None
    candidate = None
    candidate_since = 0.0
    start = time.monotonic()

    try:
        while True:
            raw = bool(sensor.pin.state)
            now = time.monotonic()
            if raw != candidate:
                candidate = raw
                candidate_since = now
            elif raw != stable and now - candidate_since >= DEBOUNCE_S:
                stable = raw
                magnet = not raw  # active low
                print(f"[{now - start:7.2f}s] level={'HIGH' if raw else 'LOW ':4} "
                      f"value={sensor.value}  ->  "
                      f"{'MAGNET' if magnet else 'no magnet'}")
            time.sleep(POLL_S)
    except KeyboardInterrupt:
        print("\nStopped.")
    finally:
        sensor.close()


if __name__ == "__main__":
    try:
        main()
    except KeyboardInterrupt:
        print("Stopped.")
