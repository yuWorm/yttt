#!/usr/bin/env python3
"""Installer safety tests; run on Linux with python3 -m unittest discover -s scripts -p test_linux_install.py."""

import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time
import unittest

if sys.platform.startswith("linux"):
    import fcntl


PATH_BLOCK_START = "# >>> yttt PATH >>>"
PATH_BLOCK_END = "# <<< yttt PATH <<<"


@unittest.skipUnless(sys.platform.startswith("linux"), "requires Linux, bash, and flock")
class LinuxInstallTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="yttt-install-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.package = self.root / "package"
        (self.package / "bin").mkdir(parents=True)
        shutil.copy2(Path(__file__).resolve().parents[1] / "packaging/linux/install.sh", self.package / "install.sh")
        # Bash remains executable under the installed name; BusyBox dispatches by argv[0].
        shutil.copy2("/bin/bash", self.package / "bin/yttt")
        self.home = self.root / "home with 'quotes"
        self.home.mkdir()
        self.env = {
            "HOME": str(self.home),
            "PATH": "/usr/bin:/bin",
            "SHELL": "/bin/bash",
        }

    def install(self, env=None):
        return subprocess.run(
            ["/bin/bash", str(self.package / "install.sh")],
            env=self.env if env is None else env,
            capture_output=True,
            text=True,
        )

    def assert_single_path_block(self, config):
        content = config.read_text()
        self.assertEqual(content.count(PATH_BLOCK_START), 1)
        self.assertEqual(content.count(PATH_BLOCK_END), 1)

    def run_fresh_bash(self, *arguments, home=None):
        environment = self.env.copy()
        if home is not None:
            environment["HOME"] = str(home)
        environment["PATH"] = "/usr/bin:/bin"
        return subprocess.run(
            ["/bin/bash", *arguments, "-c", 'yttt -c "exit 17"'],
            env=environment,
            capture_output=True,
            text=True,
        )

    def wait_for_flock_waiter(self, process):
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            if process.poll() is not None:
                _, stderr = process.communicate()
                self.fail(f"installer exited before acquiring the held lock: {stderr}")
            try:
                child_processes = Path(f"/proc/{process.pid}/task/{process.pid}/children").read_text().split()
            except FileNotFoundError:
                child_processes = []
            lock_waiters = [" ".join(line.split()) for line in Path("/proc/locks").read_text().splitlines()]
            waiting_processes = [str(process.pid), *child_processes]
            if any(f"-> FLOCK ADVISORY WRITE {pid} " in line for pid in waiting_processes for line in lock_waiters):
                return
            time.sleep(0.01)

        if process.poll() is None:
            process.terminate()
        _, stderr = process.communicate()
        self.fail(f"installer did not wait for the held lock: {stderr}")

    def test_foreign_commands_and_broken_links_are_preserved(self):
        command = self.home / ".local/bin/yttt"
        command.parent.mkdir(parents=True)
        for kind in ("file", "broken-link", "directory"):
            with self.subTest(kind=kind):
                if kind == "file":
                    command.write_bytes(b"user command")
                elif kind == "broken-link":
                    command.symlink_to("missing-user-binary")
                else:
                    command.mkdir()
                result = self.install()
                self.assertNotEqual(result.returncode, 0)
                self.assertFalse((self.home / ".local/share/yttt/desktop").exists())
                self.assertFalse((self.home / ".bashrc").exists())
                if kind == "file":
                    self.assertEqual(command.read_bytes(), b"user command")
                elif kind == "broken-link":
                    self.assertEqual(os.readlink(command), "missing-user-binary")
                else:
                    self.assertEqual(list(command.iterdir()), [])
                command.rmdir() if kind == "directory" else command.unlink()

    def test_reinstall_preserves_shell_configuration_and_executable_lookup(self):
        config = self.home / ".bashrc"
        original = b"# user configuration without trailing newline"
        config.write_bytes(original)
        first = self.install()
        self.assertEqual(first.returncode, 0, first.stderr)
        configured = config.read_bytes()
        self.assertTrue(configured.startswith(original + b"\n"))
        login_config = self.home / ".bash_profile"
        configured_login = login_config.read_bytes()
        second = self.install()
        self.assertEqual(second.returncode, 0, second.stderr)
        self.assertEqual(config.read_bytes(), configured)
        self.assertEqual(login_config.read_bytes(), configured_login)
        result = subprocess.run(
            ["/bin/bash", "-c", '. "$HOME/.bashrc"; yttt -c "exit 17"'],
            env=self.env,
            capture_output=True,
        )
        self.assertEqual(result.returncode, 17, result.stderr)

    def test_malformed_managed_blocks_fail_without_touching_user_files(self):
        config = self.home / ".bashrc"
        for content in (
            b"# <<< yttt PATH <<<\nuser data\n# >>> yttt PATH >>>\n",
            b"# >>> yttt PATH >>>\nuser data\n",
            b"# >>> yttt PATH >>>\n# <<< yttt PATH <<<\n# >>> yttt PATH >>>\n# <<< yttt PATH <<<\n",
        ):
            with self.subTest(content=content):
                config.write_bytes(content)
                self.assertNotEqual(self.install().returncode, 0)
                self.assertEqual(config.read_bytes(), content)
                self.assertFalse((self.home / ".local/share/yttt/desktop").exists())

    def test_rejects_colon_in_home_before_creating_files(self):
        colon_home = self.root / "home:contains-colon"
        colon_home.mkdir()
        environment = self.env.copy()
        environment["HOME"] = str(colon_home)

        result = self.install(environment)

        self.assertNotEqual(result.returncode, 0)
        self.assertFalse((colon_home / ".yttt-install.lock").exists())
        self.assertFalse((colon_home / ".local").exists())
        self.assertFalse((colon_home / ".bashrc").exists())

    def test_persists_bash_paths_when_the_installer_inherits_the_bin_directory(self):
        inherited_bin_directory = self.home / ".local/bin"
        environment = self.env.copy()
        environment["PATH"] = f"{inherited_bin_directory}:/usr/bin:/bin"

        result = self.install(environment)

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assert_single_path_block(self.home / ".bashrc")
        self.assert_single_path_block(self.home / ".bash_profile")
        interactive = self.run_fresh_bash("-i")
        self.assertEqual(interactive.returncode, 17, interactive.stderr)
        login = self.run_fresh_bash("--login", "-i")
        self.assertEqual(login.returncode, 17, login.stderr)

    def test_bash_login_uses_the_first_available_startup_file(self):
        candidates = (".bash_profile", ".bash_login", ".profile")
        for selected_name in candidates:
            with self.subTest(selected_name=selected_name):
                home = self.root / f"login-{selected_name}"
                home.mkdir()
                selected = home / selected_name
                original = f"# user {selected_name} without trailing newline".encode()
                selected.write_bytes(original)
                environment = self.env.copy()
                environment["HOME"] = str(home)

                result = self.install(environment)

                self.assertEqual(result.returncode, 0, result.stderr)
                configured = selected.read_bytes()
                self.assertTrue(configured.startswith(original + b"\n"))
                self.assert_single_path_block(selected)
                self.assert_single_path_block(home / ".bashrc")
                for other_name in candidates:
                    if other_name != selected_name:
                        self.assertFalse((home / other_name).exists())
                login = self.run_fresh_bash("--login", "-i", home=home)
                self.assertEqual(login.returncode, 17, login.stderr)

    def test_shell_configuration_symlinks_and_user_bytes_are_preserved(self):
        dotfiles = self.home / "dotfiles"
        dotfiles.mkdir()
        target = dotfiles / "bash-startup"
        original = b"# preserved bytes without trailing newline"
        target.write_bytes(original)
        bashrc = self.home / ".bashrc"
        bash_profile = self.home / ".bash_profile"
        bashrc.symlink_to(target)
        bash_profile.symlink_to(target)

        result = self.install()

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(bashrc.is_symlink())
        self.assertTrue(bash_profile.is_symlink())
        self.assertEqual(os.readlink(bashrc), str(target))
        self.assertEqual(os.readlink(bash_profile), str(target))
        configured = target.read_bytes()
        self.assertTrue(configured.startswith(original + b"\n"))
        self.assert_single_path_block(target)
        interactive = self.run_fresh_bash("-i")
        self.assertEqual(interactive.returncode, 17, interactive.stderr)
        login = self.run_fresh_bash("--login", "-i")
        self.assertEqual(login.returncode, 17, login.stderr)

    def test_non_bash_shells_resolve_the_command_with_custom_config_roots(self):
        cases = (
            ("zsh", {"ZDOTDIR": "zsh-dotdir"}),
            ("fish", {"XDG_CONFIG_HOME": "fish-config"}),
            ("sh", {}),
        )
        for shell, variables in cases:
            executable = shutil.which(shell)
            if executable is None:
                continue
            with self.subTest(shell=shell):
                home = self.root / f"{shell}-home"
                home.mkdir()
                environment = self.env.copy()
                environment["HOME"] = str(home)
                environment["SHELL"] = executable
                for name, relative_path in variables.items():
                    environment[name] = str(home / relative_path)
                result = self.install(environment)
                self.assertEqual(result.returncode, 0, result.stderr)
                fresh = subprocess.run(
                    [executable, "-lic", 'yttt -c "exit 17"'],
                    env=environment, capture_output=True, text=True,
                )
                self.assertEqual(fresh.returncode, 17, fresh.stderr)

    def test_concurrent_reinstalls_keep_one_managed_block_and_leave_a_reusable_lock(self):
        processes = [
            subprocess.Popen(
                ["/bin/bash", str(self.package / "install.sh")],
                env=self.env,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )
            for _ in range(12)
        ]
        results = [process.communicate(timeout=15) for process in processes]
        for process, (_, stderr) in zip(processes, results):
            self.assertEqual(process.returncode, 0, stderr)
        self.assert_single_path_block(self.home / ".bashrc")
        self.assert_single_path_block(self.home / ".bash_profile")
        self.assertTrue((self.home / ".yttt-install.lock").is_file())

        retry = self.install()

        self.assertEqual(retry.returncode, 0, retry.stderr)
        self.assert_single_path_block(self.home / ".bashrc")
        self.assert_single_path_block(self.home / ".bash_profile")

    def test_lock_covers_validation_and_stale_lock_file_allows_retry(self):
        lock_path = self.home / ".yttt-install.lock"
        lock_path.touch()
        with lock_path.open("a+") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX)
            process = subprocess.Popen(
                ["/bin/bash", str(self.package / "install.sh")],
                env=self.env,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )
            try:
                self.wait_for_flock_waiter(process)
                command = self.home / ".local/bin/yttt"
                command.parent.mkdir(parents=True)
                command.write_bytes(b"user command created while installation waits")
            finally:
                fcntl.flock(lock, fcntl.LOCK_UN)

        _, stderr = process.communicate(timeout=15)
        self.assertNotEqual(process.returncode, 0)
        self.assertEqual(command.read_bytes(), b"user command created while installation waits")
        self.assertFalse((self.home / ".local/share/yttt/desktop").exists())
        self.assertFalse((self.home / ".bashrc").exists())
        self.assertFalse((self.home / ".bash_profile").exists())

        command.unlink()
        retry = self.install()

        self.assertEqual(retry.returncode, 0, retry.stderr)
        self.assert_single_path_block(self.home / ".bashrc")
        self.assert_single_path_block(self.home / ".bash_profile")

    def test_conflicting_bin_parent_fails_before_installation_and_can_retry(self):
        bin_directory = self.home / ".local/bin"
        bin_directory.parent.mkdir()
        bin_directory.write_bytes(b"not a directory")

        failed = self.install()

        self.assertNotEqual(failed.returncode, 0)
        self.assertFalse((self.home / ".local/share/yttt/desktop").exists())
        self.assertFalse((self.home / ".bashrc").exists())
        self.assertFalse((self.home / ".bash_profile").exists())

        bin_directory.unlink()
        retry = self.install()

        self.assertEqual(retry.returncode, 0, retry.stderr)
        fresh = self.run_fresh_bash("--login", "-i")
        self.assertEqual(fresh.returncode, 17, fresh.stderr)


if __name__ == "__main__":
    unittest.main()
