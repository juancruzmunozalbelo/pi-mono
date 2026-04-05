"""Harbor agent adapter for Pi Rust."""

import os
import subprocess
import json
from pathlib import Path
from typing import Optional

try:
    from harbor.agent import BaseInstalledAgent, AgentContext
    HAS_HARBOR = True
except ImportError:
    HAS_HARBOR = False

# Also create a standalone runner that doesn't need Harbor
class PiRustRunner:
    """Standalone benchmark runner for Pi Rust (no Harbor dependency)."""

    def __init__(self, binary_path: str = "./target/release/pi", model: str = None):
        self.binary = binary_path
        self.model = model

    def run_task(self, instruction: str, timeout: int = 300) -> dict:
        """Run a single benchmark task and return results."""
        cmd = [self.binary, "-p", instruction]
        if self.model:
            cmd.extend(["-m", self.model])

        try:
            result = subprocess.run(
                cmd,
                capture_output=True,
                text=True,
                timeout=timeout,
                cwd=os.getcwd(),
            )
            return {
                "success": result.returncode == 0,
                "output": result.stdout,
                "error": result.stderr,
                "exit_code": result.returncode,
            }
        except subprocess.TimeoutExpired:
            return {
                "success": False,
                "output": "",
                "error": f"Timed out after {timeout}s",
                "exit_code": -1,
            }

if HAS_HARBOR:
    class PiRustAgent(BaseInstalledAgent):
        """Harbor agent adapter for Pi Rust."""

        @property
        def agent_id(self) -> str:
            return "pi-rust"

        def get_install_template(self) -> Optional[str]:
            return "install-pi-rust.sh.j2"

        def get_run_commands(self, ctx: AgentContext) -> list[str]:
            model_args = ""
            if ctx.model:
                model_args = f"-m {ctx.model}"

            instruction = ctx.instruction.replace('"', '\\"').replace('$', '\\$')

            return [
                f'mkdir -p {ctx.output_dir}',
                f'/usr/local/bin/pi {model_args} -p "{instruction}" > {ctx.output_dir}/output.txt 2>&1 || true',
            ]

        def populate_context(self, ctx: AgentContext) -> None:
            output_file = os.path.join(ctx.output_dir, "output.txt")
            if os.path.exists(output_file):
                with open(output_file) as f:
                    ctx.agent_output = f.read()
