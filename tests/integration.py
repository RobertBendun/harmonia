import contextlib
import requests
import signal
import subprocess
import unittest

@contextlib.contextmanager
def harmonia():
    # subprocess.run(["cargo", "build", "--bin", "harmonia"], check=True)
    with subprocess.Popen(["cargo", "run", "--bin", "harmonia"], stdout=subprocess.PIPE) as p:
        try:
            for line in p.stdout:
                # That means that Harmonia should be ready to accept connections
                if b"Listening" in line:
                    yield
                    break

            p.send_signal(signal.SIGINT)
        except:
            p.kill()


class IntegrationTesting(unittest.TestCase):
    def test_connects(self):
        with harmonia():
            response = requests.get("http://localhost:8080/midi/ports")
            assert response.status_code == 200
            print(response.text)



if __name__ == "__main__":
    unittest.main()
