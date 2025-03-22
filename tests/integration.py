import contextlib
import requests
import signal
import subprocess
import unittest


@contextlib.contextmanager
def harmonia():
    ip = "127.0.0.1"
    port = "8888"

    with subprocess.Popen(
        [
            "cargo",
            "run",
            "--bin",
            "harmonia",
            "--",
            "--ip",
            ip,
            "--port",
            port,
        ],
        stdout=subprocess.PIPE,
    ) as p:
        address = b"http://%s:%s" % (ip.encode("utf-8"), port.encode("utf-8"))

        try:
            for line in p.stdout:
                # That means that Harmonia should be ready to accept connections
                if address in line:
                    yield address
                    break

            p.send_signal(signal.SIGINT)
            p.wait(10)
        except:
            p.kill()


class IntegrationTesting(unittest.TestCase):
    def test_connects(self):
        with harmonia() as url:
            response = requests.getf(f"{url}/midi/ports")
            assert response.status_code == 200
            print(response.text)


if __name__ == "__main__":
    unittest.main()
