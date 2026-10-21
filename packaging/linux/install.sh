#!/usr/bin/env bash
set -euo pipefail

installer_marker='yttt-user-install-v1'
path_block_start='# >>> yttt PATH >>>'
path_block_end='# <<< yttt PATH <<<'

installer_locale="${LC_ALL:-${LC_MESSAGES:-${LANG:-}}}"
installer_language='en'
case "$installer_locale" in
  zh|zh[_@.-]*) installer_language='zh_CN' ;;
esac

installer_label='yttt install'
usage_text=$'Usage: ./install.sh\n\nInstalls yttt for the current user. The executable is copied to the user\'s\nXDG data directory and linked from ~/.local/bin/yttt. The installer adds\nmanaged PATH blocks to the startup configuration selected by $SHELL.'
error_unknown_argument='unknown argument: %s'
error_home_not_set='HOME is not set.'
error_home_not_absolute='HOME must be an absolute path: %s'
error_home_contains_colon='HOME must not contain ":": %s'
error_home_not_directory='HOME is not a directory: %s'
error_missing_executable='missing packaged executable: %s'
error_xdg_data_home_not_absolute='XDG_DATA_HOME must be absolute: %s'
error_cli_directory_contains_colon='CLI directory must not contain ":": %s'
error_resolve_shell_config_link='cannot resolve shell configuration link: %s'
error_bash_login_config='cannot select a usable Bash login configuration file.'
error_non_directory_parent='refusing to use non-directory parent: %s'
error_shell_config_not_regular='shell configuration is not a regular file: %s'
error_duplicate_path_block='duplicate or malformed yttt PATH block: %s'
error_malformed_path_block='malformed yttt PATH block: %s'
error_incomplete_path_block='incomplete yttt PATH block: %s'
error_non_directory_installation_path='refusing to use non-directory installation path: %s'
error_existing_installation='refusing to replace an existing installation not created by this installer: %s'
error_existing_command='refusing to replace existing command: %s'
error_flock_required='flock is required for safe installation.'
error_non_regular_lock='refusing to use non-regular installation lock: %s'
error_open_lock='cannot open installation lock: %s'
error_acquire_lock='cannot acquire installation lock: %s'
output_installed='Installed yttt: %s'
output_command_link='Command link: %s'
output_path_config_ready='CLI PATH configuration is ready in %s.'
output_path_already_persistent='CLI PATH configuration is already persistent.'
output_current_terminal='Run yttt in this terminal.'
output_new_terminal='Open a new terminal to run yttt; existing terminals keep their current PATH.'

if [[ "$installer_language" == 'zh_CN' ]]; then
  installer_label='yttt 安装'
  usage_text=$'用法：./install.sh\n\n为当前用户安装 yttt。可执行文件会复制到用户的 XDG 数据目录，并在\n~/.local/bin/yttt 创建链接。安装程序会将受管的 PATH 配置块添加到由 $SHELL 选择的启动配置中。'
  error_unknown_argument='未知参数：%s'
  error_home_not_set='未设置 HOME。'
  error_home_not_absolute='HOME 必须是绝对路径：%s'
  error_home_contains_colon='HOME 不得包含“:”字符：%s'
  error_home_not_directory='HOME 不是目录：%s'
  error_missing_executable='缺少已打包的可执行文件：%s'
  error_xdg_data_home_not_absolute='XDG_DATA_HOME 必须是绝对路径：%s'
  error_cli_directory_contains_colon='CLI 目录不得包含“:”字符：%s'
  error_resolve_shell_config_link='无法解析 shell 配置文件链接：%s'
  error_bash_login_config='无法选择可用的 Bash 登录配置文件。'
  error_non_directory_parent='拒绝使用非目录父路径：%s'
  error_shell_config_not_regular='shell 配置不是常规文件：%s'
  error_duplicate_path_block='yttt PATH 配置块重复或格式错误：%s'
  error_malformed_path_block='yttt PATH 配置块格式错误：%s'
  error_incomplete_path_block='yttt PATH 配置块不完整：%s'
  error_non_directory_installation_path='拒绝使用非目录安装路径：%s'
  error_existing_installation='拒绝替换并非由此安装程序创建的现有安装：%s'
  error_existing_command='拒绝替换现有命令：%s'
  error_flock_required='安全安装需要 flock。'
  error_non_regular_lock='拒绝使用非常规安装锁文件：%s'
  error_open_lock='无法打开安装锁：%s'
  error_acquire_lock='无法获取安装锁：%s'
  output_installed='已安装 yttt：%s'
  output_command_link='命令链接：%s'
  output_path_config_ready='已在 %s 中配置 CLI PATH。'
  output_path_already_persistent='CLI PATH 配置已持久生效。'
  output_current_terminal='可在当前终端中运行 yttt。'
  output_new_terminal='请打开新的终端再运行 yttt；现有终端会保留当前 PATH。'
fi

usage() {
  printf '%s\n' "$usage_text"
}

die() {
  printf '%s: ' "$installer_label" >&2
  printf "$1\n" "${@:2}" >&2
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
      die "$error_unknown_argument" "$1"
      ;;
  esac
fi

[[ -n "${HOME:-}" ]] || die "$error_home_not_set"
case "$HOME" in
  /*) ;;
  *) die "$error_home_not_absolute" "$HOME" ;;
esac
case "$HOME" in
  *:*) die "$error_home_contains_colon" "$HOME" ;;
esac
[[ -d "$HOME" ]] || die "$error_home_not_directory" "$HOME"

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
source_binary="$script_dir/bin/yttt"
[[ -f "$source_binary" && -x "$source_binary" ]] || die "$error_missing_executable" "$source_binary"

install_dir="${XDG_DATA_HOME:-$HOME/.local/share}/yttt/desktop"
bin_dir="$HOME/.local/bin"
case "$install_dir" in
  /*) ;;
  *) die "$error_xdg_data_home_not_absolute" "${XDG_DATA_HOME}" ;;
esac
case "$bin_dir" in
  *:*) die "$error_cli_directory_contains_colon" "$bin_dir" ;;
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
      || die "$error_resolve_shell_config_link" "$config_file"
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

  die "$error_bash_login_config"
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
        || die "$error_non_directory_parent" "$directory_path"
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
        || die "$error_shell_config_not_regular" "$config_file"
      while IFS= read -r line || [[ -n "$line" ]]; do
        if [[ "$line" == "$path_block_start" ]]; then
          [[ "$state" -eq 0 ]] \
            || die "$error_duplicate_path_block" "$config_file"
          state=1
        elif [[ "$line" == "$path_block_end" ]]; then
          [[ "$state" -eq 1 ]] \
            || die "$error_malformed_path_block" "$config_file"
          state=2
        fi
      done < "$config_target"
      [[ "$state" -ne 1 ]] \
        || die "$error_incomplete_path_block" "$config_file"
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
      || die "$error_non_directory_installation_path" "$install_dir"
    [[ -f "$install_marker_file" ]] \
      || die "$error_existing_installation" "$install_dir"
    [[ "$(<"$install_marker_file")" == "$installer_marker" ]] \
      || die "$error_existing_installation" "$install_dir"
  fi
}

validate_command_path() {
  if [[ -e "$command_path" || -L "$command_path" ]]; then
    [[ -L "$command_path" && "$(readlink -- "$command_path")" == "$installed_binary" ]] \
      || die "$error_existing_command" "$command_path"
  fi
}

command -v flock >/dev/null 2>&1 \
  || die "$error_flock_required"
lock_file="$HOME/.yttt-install.lock"
if [[ -e "$lock_file" || -L "$lock_file" ]]; then
  [[ -f "$lock_file" && ! -L "$lock_file" ]] \
    || die "$error_non_regular_lock" "$lock_file"
fi
if ! exec {install_lock_fd}>> "$lock_file"; then
  die "$error_open_lock" "$lock_file"
fi
flock "$install_lock_fd" \
  || die "$error_acquire_lock" "$lock_file"

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

printf "$output_installed\n" "$installed_binary"
printf "$output_command_link\n" "$command_path"
if [[ "$path_config_changes" -gt 0 ]]; then
  for ((path_config_index = 0; path_config_index < ${#path_config_files[@]}; path_config_index++)); do
    if [[ "${path_config_blocks[path_config_index]}" -eq 0 ]]; then
      printf "$output_path_config_ready\n" "${path_config_files[path_config_index]}"
    fi
  done
else
  printf '%s\n' "$output_path_already_persistent"
fi
if path_contains "$bin_dir"; then
  printf '%s\n' "$output_current_terminal"
else
  printf '%s\n' "$output_new_terminal"
fi
