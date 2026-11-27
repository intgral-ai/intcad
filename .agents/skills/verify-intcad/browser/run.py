"""Run Bombadil against the bundled Assistant page using a local desktop-host fixture."""

import argparse
import json
from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
import shutil
import subprocess
import tempfile
from threading import Thread


def report_coverage(trace):
    steps = 0
    sends = 0
    routes = set()
    with trace.open() as records:
        for line in records:
            step = json.loads(line)
            steps += 1
            for snapshot in step["snapshots"]:
                if snapshot["name"] == "composer" and snapshot["value"]:
                    ui = snapshot["value"]
                    if ui["sends"] > sends:
                        if ui["clickedSend"]:
                            routes.add("click")
                        if ui["pressedEnter"]:
                            routes.add("Enter")
                        sends = ui["sends"]
    print(f"Observed {steps} browser states and {sends} sends to the fixture.")
    if routes != {"click", "Enter"}:
        raise SystemExit("Missing coverage: the page must complete both submit routes.")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True, help="fresh evidence directory")
    parser.add_argument("--port", type=int, default=18761, help="use the same port for replay")
    parser.add_argument("--reproduce", type=Path, help="replay a saved trace.jsonl")
    args = parser.parse_args()
    here = Path(__file__).resolve().parent
    with tempfile.TemporaryDirectory() as temp:
        site = Path(temp)
        dist = here.parents[3] / "packages" / "assistant-web" / "dist"
        shutil.copytree(dist, site, dirs_exist_ok=True)
        shutil.copyfile(here / "host.js", site / "host.js")
        index = site / "index.html"
        page = index.read_text()
        marker = '<script type="module"'
        if page.count(marker) != 1:
            parser.error("expected one module entry point in the bundled Assistant page")
        index.write_text(page.replace(marker, '<script src="./host.js"></script>\n    ' + marker))
        handler = partial(SimpleHTTPRequestHandler, directory=temp)
        with ThreadingHTTPServer(("127.0.0.1", args.port), handler) as server:
            Thread(target=server.serve_forever, daemon=True).start()
            try:
                mode = (["--reproduce", str(args.reproduce)] if args.reproduce else
                        ["--time-limit", "25s", "--exit-on-violation"])
                command = ["bombadil", "browser", "test", "--headless", *mode,
                           "--output-path", str(args.output),
                           f"http://127.0.0.1:{server.server_port}", str(here / "composer.ts")]
                result = subprocess.run(command, timeout=120)
                if result.returncode == 0 and not args.reproduce:
                    report_coverage(args.output / "trace.jsonl")
                return result.returncode
            finally:
                server.shutdown()


if __name__ == "__main__":
    raise SystemExit(main())
