"""Shared integration-test support; failed cleanup is never silently ignored."""
import os
import tempfile
import time


class TemporaryDirectory(tempfile.TemporaryDirectory):
    def cleanup(self):
        # TerminateProcess and closing a Windows job initiate child termination;
        # the last kernel file handle can outlive the gateway's process handle.
        deadline = time.monotonic() + (5 if os.name == 'nt' else 0)
        while True:
            try:
                return super().cleanup()
            except PermissionError:
                if time.monotonic() >= deadline:
                    raise
                time.sleep(0.05)
