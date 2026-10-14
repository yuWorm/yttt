#!/usr/bin/env bash
set -euo pipefail

installer_marker='yttt-user-install-v1'
path_block_start='# >>> yttt PATH >>>'
path_block_end='# <<< yttt PATH <<<'

usage() {
  cat <<'EOF'
Usage: ./install.sh

Installs yttt for the current user. The executable is copied to the user's
XDG data directory and linked from ~/.local/bin/yttt. The installer adds
managed PATH blocks to the startup configuration selected by $SHELL.
EOF
}

die() {
  printf 'yttt install: %s\n' "$*" >&2
  exit 1
}

if [[ $# -gt 0 ]]; then
  case "$1" in
    -h|--help)
      usage
      exit 0
      ;;
    *)
      usage >&2
      die "unknown argument: $1"
      ;;
  esac
fi

[[ -n "${HOME:-}" ]] || die 'HOME is not set.'
case "$HOME" in
  /*) ;;
  *) die "HOME must be an absolute path: $HOME" ;;
esac
case "$HOME" in
  *:*) die "HOME must not contain ':': $HOME" ;;
esac
[[ -d "$HOME" ]] || die "HOME is not a directory: $HOME"

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
source_binary="$script_dir/bin/yttt"
[[ -f "$source_binary" && -x "$source_binary" ]] || die "missing packaged executable: $source_binary"

install_dir="${XDG_DATA_HOME:-$HOME/.local/share}/yttt/desktop"
bin_dir="$HOME/.local/bin"
case "$install_dir" in
  /*) ;;
  *) die "XDG_DATA_HOME must be absolute: ${XDG_DATA_HOME}" ;;
esac
case "$bin_dir" in
  *:*) die "CLI directory must not contain ':': $bin_dir" ;;
esac

installed_binary="$install_dir/yttt"
install_marker_file="$install_dir/.yttt-install"
command_path="$bin_dir/yttt"

path_contains() {
  local candidate="$1"
  local entry
  local IFS=:
  local -a entries
  read -r -a entries <<< "${PATH-}"
  for entry in "${entries[@]}"; do
    while [[ "$entry" != / && "$entry" == */ ]]; do
      entry="${entry%/}"
    done
    [[ "$entry" == "$candidate" ]] && return 0
  done
  return 1
}

path_config_files=()
path_config_targets=()
path_config_styles=()
path_config_blocks=()
path_config_changes=0

add_path_config() {
  local config_file="$1"
  local config_style="$2"
  local config_target="$config_file"
  local index

  if [[ -L "$config_file" ]]; then
    config_target="$(readlink -f -- "$config_file")" \
      || die "cannot resolve shell configuration link: $config_file"
  fi

  for ((index = 0; index < ${#path_config_targets[@]}; index++)); do
    [[ "${path_config_targets[index]}" == "$config_target" ]] && return 0
  done

  path_config_files+=("$config_file")
  path_config_targets+=("$config_target")
  path_config_styles+=("$config_style")
  path_config_blocks+=(0)
}

select_bash_login_config() {
  local config_file

  for config_file in "$HOME/.bash_profile" "$HOME/.bash_login" "$HOME/.profile"; do
    if [[ -f "$config_file" ]]; then
      add_path_config "$config_file" 'sh'
      return 0
    fi
  done

  for config_file in "$HOME/.bash_profile" "$HOME/.bash_login" "$HOME/.profile"; do
    if [[ ! -e "$config_file" && ! -L "$config_file" ]]; then
      add_path_config "$config_file" 'sh'
      return 0
    fi
  done

  die 'cannot select a usable Bash login configuration file.'
}

select_path_configs() {
  case "${SHELL:-sh}" in
    */bash|bash)
      # Interactive non-login shells read .bashrc. Login shells read only the
      # first existing file among .bash_profile, .bash_login, and .profile.
      add_path_config "$HOME/.bashrc" 'sh'
      select_bash_login_config
      ;;
    */zsh|zsh)
      add_path_config "${ZDOTDIR:-$HOME}/.zshrc" 'sh'
      ;;
    */fish|fish)
      add_path_config "${XDG_CONFIG_HOME:-$HOME/.config}/fish/conf.d/yttt-path.fish" 'fish'
      ;;
    *)
      add_path_config "$HOME/.profile" 'sh'
      ;;
  esac
}

validate_directory_path() {
  local directory_path="$1"

  while :; do
    if [[ -e "$directory_path" || -L "$directory_path" ]]; then
      [[ -d "$directory_path" ]] \
        || die "refusing to use non-directory parent: $directory_path"
    fi

    case "$directory_path" in
      /|.) break ;;
      */*)
        directory_path="${directory_path%/*}"
        [[ -n "$directory_path" ]] || directory_path='/'
        ;;
      *) break ;;
    esac
  done
}

validate_path_configs() {
  local index
  local line
  local state
  local config_file
  local config_target

  for ((index = 0; index < ${#path_config_targets[@]}; index++)); do
    config_file="${path_config_files[index]}"
    config_target="${path_config_targets[index]}"
    validate_directory_path "$(dirname -- "$config_target")"

    state=0
    if [[ -e "$config_target" || -L "$config_target" ]]; then
      [[ -f "$config_target" ]] \
        || die "shell configuration is not a regular file: $config_file"
      while IFS= read -r line || [[ -n "$line" ]]; do
        if [[ "$line" == "$path_block_start" ]]; then
          [[ "$state" -eq 0 ]] \
            || die "duplicate or malformed yttt PATH block: $config_file"
          state=1
        elif [[ "$line" == "$path_block_end" ]]; then
          [[ "$state" -eq 1 ]] \
            || die "malformed yttt PATH block: $config_file"
          state=2
        fi
      done < "$config_target"
      [[ "$state" -ne 1 ]] \
        || die "incomplete yttt PATH block: $config_file"
    fi

    if [[ "$state" -eq 2 ]]; then
      path_config_blocks[index]=1
    else
      path_config_changes=$((path_config_changes + 1))
    fi
  done
}

write_path_config() {
  local index="$1"
  local config_target="${path_config_targets[index]}"
  local config_style="${path_config_styles[index]}"

  [[ "${path_config_blocks[index]}" -eq 0 ]] || return 0
  mkdir -p -- "$(dirname -- "$config_target")"
  # Append only: preserve the user's bytes, permissions, symlinks, and managed block.
  if [[ "$config_style" == 'fish' ]]; then
    cat >> "$config_target" <<'EOF'

# >>> yttt PATH >>>
set -l yttt_bin_dir "$HOME/.local/bin"
if not contains -- "$yttt_bin_dir" $PATH
    set -gx PATH "$yttt_bin_dir" $PATH
end
set -e yttt_bin_dir
# <<< yttt PATH <<<
EOF
  else
    cat >> "$config_target" <<'EOF'

# >>> yttt PATH >>>
yttt_bin_dir="$HOME/.local/bin"
case ":${PATH-}:" in
  *":${yttt_bin_dir}:"*) ;;
  *) export PATH="${yttt_bin_dir}${PATH:+:${PATH}}" ;;
esac
unset yttt_bin_dir
# <<< yttt PATH <<<
EOF
  fi
}

validate_installation() {
  if [[ -e "$install_dir" || -L "$install_dir" ]]; then
    [[ -d "$install_dir" && ! -L "$install_dir" ]] \
      || die "refusing to use non-directory installation path: $install_dir"
    [[ -f "$install_marker_file" ]] \
      || die "refusing to replace an existing installation not created by this installer: $install_dir"
    [[ "$(<"$install_marker_file")" == "$installer_marker" ]] \
      || die "refusing to replace an existing installation not created by this installer: $install_dir"
  fi
}

validate_command_path() {
  if [[ -e "$command_path" || -L "$command_path" ]]; then
    [[ -L "$command_path" && "$(readlink -- "$command_path")" == "$installed_binary" ]] \
      || die "refusing to replace existing command: $command_path"
  fi
}

command -v flock >/dev/null 2>&1 \
  || die 'flock is required for safe installation.'
lock_file="$HOME/.yttt-install.lock"
if [[ -e "$lock_file" || -L "$lock_file" ]]; then
  [[ -f "$lock_file" && ! -L "$lock_file" ]] \
    || die "refusing to use non-regular installation lock: $lock_file"
fi
if ! exec {install_lock_fd}>> "$lock_file"; then
  die "cannot open installation lock: $lock_file"
fi
flock "$install_lock_fd" \
  || die "cannot acquire installation lock: $lock_file"

# Validate mutable user state only after all installers for this user serialize.
select_path_configs
validate_directory_path "$install_dir"
validate_directory_path "$bin_dir"
validate_path_configs
validate_installation
validate_command_path

install_dir_is_new=0
if [[ ! -e "$install_dir" && ! -L "$install_dir" ]]; then
  install_dir_is_new=1
fi

binary_temporary=''
marker_temporary=''
install_dir_created=0
installation_owned=0
cleanup() {
  [[ -z "$binary_temporary" ]] || rm -f -- "$binary_temporary"
  [[ -z "$marker_temporary" ]] || rm -f -- "$marker_temporary"
  if [[ "$install_dir_created" -eq 1 && "$installation_owned" -eq 0 ]]; then
    rmdir -- "$install_dir" 2>/dev/null || true
  fi
}
trap cleanup EXIT

if [[ "$install_dir_is_new" -eq 1 ]]; then
  mkdir -p -- "$install_dir"
  install_dir_created=1
fi
mkdir -p -- "$bin_dir"

if [[ "$install_dir_is_new" -eq 1 ]]; then
  marker_temporary="$(mktemp "${install_dir}/.yttt-install.XXXXXX")"
  printf '%s\n' "$installer_marker" > "$marker_temporary"
  mv -f -- "$marker_temporary" "$install_marker_file"
  marker_temporary=''
fi
installation_owned=1

binary_temporary="$(mktemp "${install_dir}/.yttt.XXXXXX")"
install -m 755 "$source_binary" "$binary_temporary"
mv -f -- "$binary_temporary" "$installed_binary"
binary_temporary=''

if [[ ! -L "$command_path" ]]; then
  ln -s -- "$installed_binary" "$command_path"
fi

for ((path_config_index = 0; path_config_index < ${#path_config_targets[@]}; path_config_index++)); do
  write_path_config "$path_config_index"
done

printf 'Installed yttt: %s\n' "$installed_binary"
printf 'Command link: %s\n' "$command_path"
if [[ "$path_config_changes" -gt 0 ]]; then
  for ((path_config_index = 0; path_config_index < ${#path_config_files[@]}; path_config_index++)); do
    if [[ "${path_config_blocks[path_config_index]}" -eq 0 ]]; then
      printf 'CLI PATH configuration is ready in %s.\n' "${path_config_files[path_config_index]}"
    fi
  done
else
  printf 'CLI PATH configuration is already persistent.\n'
fi
if path_contains "$bin_dir"; then
  printf 'Run yttt in this terminal.\n'
else
  printf 'Open a new terminal to run yttt; existing terminals keep their current PATH.\n'
fi
