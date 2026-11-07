#!/bin/sh
# Installs the yttt relay control plane: the console, accounts and billing
# that relay nodes report to.
#
#   curl -fsSL https://github.com/yuWorm/yttt/releases/download/v__VERSION__/install-control.sh -o install-control.sh
#   sudo sh install-control.sh
#
# It asks for a language, how to install (Docker or straight onto this
# Linux server), which database (SQLite or PostgreSQL), and how browsers
# reach the console (Caddy on ports 80/443 with automatic certificates, or
# only the service, on an address of this machine, for an existing reverse
# proxy). Every question has an option, and --yes takes the defaults, for
# running it unattended:
#
#   --method docker|native     --database sqlite|postgres
#   --expose caddy|service     --domain NAME  --email ADDRESS
#   --listen ADDRESS:PORT      --public-url URL  --behind-proxy yes|no
#   --relay-domain NAME | --no-relay   (with Caddy: also run a relay node here)
#   --admin-email ADDRESS      (password from YTTT_ADMIN_PASSWORD)
#   --signup open|closed       --dir DIR (Docker; default /opt/yttt-control)
#   --lang en|zh               (or YTTT_LANG)
#
# Installing leaves /usr/local/bin/yttt-control-install, this script. Run
# without options it shows the status and offers to upgrade, reconfigure or
# uninstall; --upgrade, --reconfigure and --uninstall [--purge] skip the
# question. Upgrading finds the latest release (--version X for another),
# checks its installer against the release's SHA256SUMS and hands over to
# it; it never goes back to an older release, whose program could not read
# the newer database (--force otherwise). Reconfiguring keeps the generated
# secrets: new ones would invalidate every relay ticket and node.
#
# Downloads are this release's own files, checked against its SHA256SUMS,
# which comes from the same place: the trust is in HTTPS to GitHub.
# For testing: --asset-dir DIR installs from local copies of the release
# files (and upgrades to the installer there), --image NAME uses another
# Docker image, and YTTT_CADDY_TEST="https_port http_port" makes Caddy use
# its own CA on those ports instead of Let's Encrypt.
#
# yttt-installer-protocol: 1

set -eu

VERSION='__VERSION__'
REPO='yuWorm/yttt'
CADDY_VERSION='2.10.0'

NATIVE_CONFIG_DIR=/etc/yttt-control
NATIVE_CONFIG=$NATIVE_CONFIG_DIR/control.toml
NATIVE_DATA=/var/lib/yttt-control
NATIVE_SHARE=/usr/share/yttt-control
NATIVE_BIN=/usr/local/bin/yttt-control
NATIVE_UNIT=/etc/systemd/system/yttt-control.service
CADDY_UNIT=/etc/systemd/system/caddy.service
CERT_SYNC=/usr/local/bin/yttt-relay-cert-sync
CERT_UNIT=/etc/systemd/system/yttt-relay-cert
MANAGER=/usr/local/bin/yttt-control-install
# Where a Docker install lives, for the management command to find it.
LOCATION=$NATIVE_CONFIG_DIR/install-location
SERVICE_USER=yttt-control
# The image runs as nobody.
IMAGE_UID=65534
UI_NAME=install-control

# >>> ui.sh
# The installers' terminal interface, shared by install-control.sh and the
# relay node's relay.sh. Edit packaging/installer/ui.sh and run
# `python3 scripts/sync_installer_ui.py --write`; CI checks the copies match.
#
# POSIX sh only (dash, bash, busybox ash). Every internal name starts with
# _ui_; the script provides msg_en and msg_zh, and UI_NAME for errors.

UI_MODE=none
UI_LANG=en
UI_UTF8=0
_ui_color=0
_ui_stty=''
_ui_drawn=0
_ui_cleanups=''
_ui_cleaned=0
_ui_spinning=''

# --- Exit paths ---------------------------------------------------------------

# Registers a command to run once on any exit, after the terminal is restored.
ui_on_exit() {
    _ui_cleanups="$_ui_cleanups
$1"
}

_ui_restore_terminal() {
    if [ -n "$_ui_stty" ]; then
        { stty "$_ui_stty" </dev/tty; } 2>/dev/null || true
    fi
    if [ "$_ui_drawn" = 1 ]; then
        printf '\033[?25h\033[?7h' >/dev/tty 2>/dev/null || true
        _ui_drawn=0
    fi
}

_ui_cleanup() {
    [ "$_ui_cleaned" = 0 ] || return 0
    _ui_cleaned=1
    # A command still running under a spinner stops with the installer.
    [ -z "$_ui_spinning" ] || kill "$_ui_spinning" 2>/dev/null || true
    _ui_restore_terminal
    _ui_old_ifs=$IFS
    IFS='
'
    set -f
    for _ui_line in $_ui_cleanups; do
        eval "$_ui_line" || true
    done
    set +f
    IFS=$_ui_old_ifs
}

# exec does not run the EXIT trap: clean up first.
ui_before_exec() {
    _ui_cleanup
    trap - EXIT INT TERM HUP
}

# From here on nothing is asked, and a dropped SSH session must not stop
# the install halfway.
ui_detach() {
    trap '' HUP
}

# --- Setup --------------------------------------------------------------------

_ui_has() {
    command -v "$1" >/dev/null 2>&1
}

# ui_init YES: decides how to talk to the person, then sets the traps.
ui_init() {
    trap '_ui_cleanup' EXIT
    trap '_ui_cleanup; exit 130' INT
    trap '_ui_cleanup; exit 143' TERM
    UI_MODE=none
    if [ "$1" != 1 ] && { [ -t 0 ] || [ -t 1 ] || [ -t 2 ]; } && { : </dev/tty; } 2>/dev/null; then
        UI_MODE=plain
        case "${TERM:-dumb}" in
            dumb | '') ;;
            *)
                if _ui_stty=$( { stty -g </dev/tty; } 2>/dev/null) && [ -n "$_ui_stty" ] \
                    && _ui_has dd && _ui_has od; then
                    UI_MODE=rich
                else
                    _ui_stty=''
                fi
                ;;
        esac
    fi
    case "${LC_ALL:-}${LC_CTYPE:-}${LANG:-}" in
        *[Uu][Tt][Ff]-8* | *[Uu][Tt][Ff]8*) UI_UTF8=1 ;;
        *)
            if _ui_has locale && [ "$(locale charmap 2>/dev/null)" = UTF-8 ]; then
                UI_UTF8=1
            fi
            ;;
    esac
    _ui_color=0
    if [ "$UI_MODE" = rich ] && [ -z "${NO_COLOR:-}" ]; then
        _ui_color=1
    fi
}

# Whether questions can be asked.
ui_interactive() {
    [ "$UI_MODE" != none ]
}

# The language: YTTT_LANG, else a question whose default follows the locale.
ui_choose_language() {
    case "${LC_ALL:-${LC_MESSAGES:-${LANG:-}}}" in
        zh*) _ui_guess=zh ;;
        *) _ui_guess=en ;;
    esac
    case "${YTTT_LANG:-}" in
        en | zh) UI_LANG=$YTTT_LANG ;;
        *)
            UI_LANG=$_ui_guess
            if ui_interactive; then
                _ui_lang=''
                if [ "$UI_UTF8" = 1 ]; then
                    _ui_zh='中文|Chinese'
                else
                    _ui_zh='Zhongwen|Chinese; needs a terminal that shows UTF-8'
                fi
                UI_LANG=en
                ui_select _ui_lang "Language / 语言" "$_ui_guess" "en|English" "zh|$_ui_zh"
                UI_LANG=$_ui_lang
            fi
            ;;
    esac
    # Choosing Chinese says the terminal shows it, whatever the locale says.
    [ "$UI_LANG" = en ] || UI_UTF8=1
    YTTT_LANG=$UI_LANG
    export YTTT_LANG
}

# --- Text -----------------------------------------------------------------------

# t KEY [ARG...]: the message in the chosen language, %s replaced in order.
t() {
    _ui_key=$1
    shift
    _ui_fmt=$("msg_$UI_LANG" "$_ui_key")
    [ -n "$_ui_fmt" ] || _ui_fmt=$(msg_en "$_ui_key")
    [ -n "$_ui_fmt" ] || _ui_fmt=$(_ui_msg "$_ui_key")
    [ -n "$_ui_fmt" ] || _ui_fmt=$_ui_key
    # shellcheck disable=SC2059
    printf "$_ui_fmt" "$@"
}

# The interface's own messages.
_ui_msg() {
    case "$UI_LANG:$1" in
        zh:ui.keys) _ui_m='↑↓ 移动 · 回车确认 · 数字直选 · q 退出' ;;
        zh:ui.choose) _ui_m='选择 [默认 %s]: ' ;;
        zh:ui.pick) _ui_m='请输入列表中的编号。' ;;
        zh:ui.again) _ui_m='再输入一次' ;;
        zh:ui.differ) _ui_m='两次输入不一致，请重试。' ;;
        zh:ui.short) _ui_m='至少 %s 个字符。' ;;
        zh:ui.yes) _ui_m='是' ;;
        zh:ui.no) _ui_m='否' ;;
        zh:ui.failed) _ui_m='失败；日志的最后几行：' ;;
        zh:ui.port.q) _ui_m='端口' ;;
        zh:ui.port.bad) _ui_m='端口是 1 到 65535 之间的数字。' ;;
        zh:ui.port.used) _ui_m='端口 %s（%s）已被 %s 占用。' ;;
        zh:ui.port.someone) _ui_m='其他程序' ;;
        zh:ui.port.unknown) _ui_m='没有 ss 或 netstat，无法检查端口是否被占用。' ;;
        zh:ui.listen.q) _ui_m='监听地址' ;;
        zh:ui.listen.loopback) _ui_m='仅本机' ;;
        zh:ui.listen.loopback.hint) _ui_m='同一台机器上的 nginx、Caddy 等代理转发到这里' ;;
        zh:ui.listen.bridge) _ui_m='Docker 网桥' ;;
        zh:ui.listen.bridge.hint) _ui_m='仅本机与容器可达；SafeLine 等拒绝 127.0.0.1 的代理用它' ;;
        zh:ui.listen.private) _ui_m='内网' ;;
        zh:ui.listen.public) _ui_m='公网' ;;
        zh:ui.listen.bridged) _ui_m='网桥' ;;
        zh:ui.listen.all) _ui_m='所有网卡' ;;
        zh:ui.listen.all.hint) _ui_m='任何能访问这台机器的人都能直接连接' ;;
        zh:ui.listen.custom) _ui_m='其他地址…' ;;
        zh:ui.listen.custom.q) _ui_m='地址（IPv4，或 [IPv6]）' ;;
        zh:ui.listen.custom.bad) _ui_m='%s 不是这台机器的地址。' ;;
        zh:ui.listen.custom.unchecked) _ui_m='没有 ip 命令，无法确认 %s 是本机地址。' ;;
        zh:ui.listen.exposed) _ui_m='%s 能从外部直接访问，会绕过前面的代理；确认防火墙只放行代理。' ;;
        zh:ui.listen.bad) _ui_m='监听地址应为 地址:端口，例如 127.0.0.1:8080 或 [::1]:8080：%s' ;;
        *:ui.keys) _ui_m='↑↓ move · Enter select · a number picks · q quits' ;;
        *:ui.choose) _ui_m='Choose [default %s]: ' ;;
        *:ui.pick) _ui_m='Enter a number from the list.' ;;
        *:ui.again) _ui_m='Again' ;;
        *:ui.differ) _ui_m='They differ; try again.' ;;
        *:ui.short) _ui_m='Use at least %s characters.' ;;
        *:ui.yes) _ui_m='Yes' ;;
        *:ui.no) _ui_m='No' ;;
        *:ui.failed) _ui_m='failed; the last lines of its log:' ;;
        *:ui.port.q) _ui_m='Port' ;;
        *:ui.port.bad) _ui_m='A port is a number from 1 to 65535.' ;;
        *:ui.port.used) _ui_m='Port %s on %s is in use by %s.' ;;
        *:ui.port.someone) _ui_m='another program' ;;
        *:ui.port.unknown) _ui_m='Neither ss nor netstat is here, so ports cannot be checked.' ;;
        *:ui.listen.q) _ui_m='Listen on' ;;
        *:ui.listen.loopback) _ui_m='This machine only' ;;
        *:ui.listen.loopback.hint) _ui_m='nginx, Caddy or another proxy on this machine forwards here' ;;
        *:ui.listen.bridge) _ui_m='Docker bridge' ;;
        *:ui.listen.bridge.hint) _ui_m='only this machine and its containers; for proxies that refuse 127.0.0.1, such as SafeLine' ;;
        *:ui.listen.private) _ui_m='private network' ;;
        *:ui.listen.public) _ui_m='public' ;;
        *:ui.listen.bridged) _ui_m='bridge' ;;
        *:ui.listen.all) _ui_m='All interfaces' ;;
        *:ui.listen.all.hint) _ui_m='anyone who reaches this machine connects directly' ;;
        *:ui.listen.custom) _ui_m='Another address…' ;;
        *:ui.listen.custom.q) _ui_m='Address (IPv4, or [IPv6])' ;;
        *:ui.listen.custom.bad) _ui_m='%s is not an address of this machine.' ;;
        *:ui.listen.custom.unchecked) _ui_m='Without the ip command, %s cannot be checked as an address of this machine.' ;;
        *:ui.listen.exposed) _ui_m='%s is reachable from outside, bypassing the proxy in front; let the firewall admit only the proxy.' ;;
        *:ui.listen.bad) _ui_m='A listen address is ADDRESS:PORT, such as 127.0.0.1:8080 or [::1]:8080: %s' ;;
        *) _ui_m='' ;;
    esac
    printf '%s' "$_ui_m"
}

_ui_sym() {
    if [ "$UI_UTF8" = 1 ]; then
        case "$1" in
            pointer) printf '❯' ;; ok) printf '✔' ;; fail) printf '✘' ;; warn) printf '▲' ;;
            step) printf '◆' ;; top) printf '╭─' ;; side) printf '│' ;; bottom) printf '╰─' ;;
            ask) printf '?' ;;
        esac
    else
        case "$1" in
            pointer) printf '>' ;; ok) printf 'ok' ;; fail) printf 'x' ;; warn) printf '!' ;;
            step) printf '==>' ;; top) printf '+-' ;; side) printf '|' ;; bottom) printf '+-' ;;
            ask) printf '?' ;;
        esac
    fi
}

# _ui_c CODE TEXT: TEXT in an SGR colour when colours are on.
_ui_c() {
    if [ "$_ui_color" = 1 ]; then
        printf '\033[%sm%s\033[0m' "$1" "$2"
    else
        printf '%s' "$2"
    fi
}

# --- Output ---------------------------------------------------------------------

ui_banner() {
    [ "$UI_MODE" != none ] || return 0
    {
        printf '\n'
        _ui_c '36;1' '        _   _   _'; printf '\n'
        _ui_c '36;1' '  _   _| |_| |_| |_'; printf '\n'
        _ui_c '36;1' ' | | | | __| __| __|'; printf '   %s\n' "$(_ui_c 1 "$1")"
        _ui_c '36;1' ' | |_| | |_| |_| |_'; printf '   %s\n' "$(_ui_c 2 "$2")"
        _ui_c '36;1' '  \__, |\__|\__|\__|'; printf '\n'
        _ui_c '36;1' '  |___/'; printf '\n\n'
    } >/dev/tty
}

ui_step() {
    printf '%s %s\n' "$(_ui_c 36 "$(_ui_sym step)")" "$*"
}

ui_ok() {
    printf '%s %s\n' "$(_ui_c 32 "$(_ui_sym ok)")" "$*"
}

ui_note() {
    printf '  %s\n' "$*"
}

ui_warn() {
    printf '%s %s\n' "$(_ui_c 33 "$(_ui_sym warn)")" "$*" >&2
}

die() {
    _ui_restore_terminal
    printf '%s %s: %s\n' "$(_ui_c 31 "$(_ui_sym fail)")" "$UI_NAME" "$*" >&2
    exit 1
}

# ui_box TITLE LINE...: a summary with a rule on its left only, since CJK
# text has no byte width to align a right edge to.
ui_box() {
    printf '\n%s %s\n' "$(_ui_c 36 "$(_ui_sym top)")" "$(_ui_c 1 "$1")"
    shift
    for _ui_line in "$@"; do
        printf '%s %s\n' "$(_ui_c 36 "$(_ui_sym side)")" "$_ui_line"
    done
    printf '%s\n' "$(_ui_c 36 "$(_ui_sym bottom)")"
}

# ui_spin LABEL COMMAND...: runs COMMAND with its output in a log, showing a
# spinner on a terminal; on failure prints the log's tail and returns its status.
ui_spin() {
    _ui_label=$1
    shift
    _ui_log=$(mktemp)
    if [ "$UI_MODE" = rich ]; then
        "$@" >"$_ui_log" 2>&1 &
        _ui_pid=$!
        _ui_spinning=$_ui_pid
        if [ "$UI_UTF8" = 1 ]; then _ui_frames='⠋ ⠙ ⠹ ⠸ ⠼ ⠴ ⠦ ⠧ ⠇ ⠏'; else _ui_frames='- \ | /'; fi
        printf '\033[?25l' >/dev/tty
        _ui_drawn=1
        while kill -0 "$_ui_pid" 2>/dev/null; do
            for _ui_frame in $_ui_frames; do
                kill -0 "$_ui_pid" 2>/dev/null || break
                printf '\r%s %s\033[K' "$(_ui_c 36 "$_ui_frame")" "$_ui_label" >/dev/tty
                sleep 0.1 2>/dev/null || sleep 1
            done
        done
        _ui_status=0
        wait "$_ui_pid" || _ui_status=$?
        _ui_spinning=''
        printf '\r\033[K\033[?25h' >/dev/tty
        _ui_drawn=0
    else
        printf '%s %s\n' "$(_ui_sym step)" "$_ui_label"
        _ui_status=0
        # A subshell, as on a terminal: a failing step shows its log.
        ("$@") >"$_ui_log" 2>&1 || _ui_status=$?
    fi
    if [ "$_ui_status" = 0 ]; then
        [ "$UI_MODE" != rich ] || ui_ok "$_ui_label"
    else
        printf '%s %s: %s\n' "$(_ui_c 31 "$(_ui_sym fail)")" "$_ui_label" "$(t ui.failed)" >&2
        tail -n 20 "$_ui_log" >&2
    fi
    rm -f "$_ui_log"
    return "$_ui_status"
}

# --- Questions ------------------------------------------------------------------

# One key from the terminal: up, down, enter, quit, digit N, or none.
_ui_byte() {
    dd bs=1 count=1 2>/dev/null </dev/tty | od -An -tu1 | tr -d ' \n'
}

_ui_read_key() {
    _ui_b=$(_ui_byte)
    case "$_ui_b" in
        27)
            # Arrow keys arrive as ESC [ A or ESC O A, perhaps split over SSH.
            stty min 0 time 3 </dev/tty
            _ui_b2=$(_ui_byte)
            _ui_key=none
            case "$_ui_b2" in
                91 | 79)
                    _ui_b3=$(_ui_byte)
                    case "$_ui_b3" in
                        65) _ui_key=up ;;
                        66) _ui_key=down ;;
                        4[89] | 5[0-7])
                            # ESC [ 5 ~ and the like: drop the rest.
                            _ui_n=0
                            while [ $_ui_n -lt 8 ]; do
                                _ui_b4=$(_ui_byte)
                                case "$_ui_b4" in '' | 126) break ;; esac
                                _ui_n=$((_ui_n + 1))
                            done
                            ;;
                    esac
                    ;;
            esac
            stty min 1 time 0 </dev/tty
            echo "$_ui_key"
            ;;
        10 | 13) echo enter ;;
        107) echo up ;;
        106) echo down ;;
        113 | 81) echo quit ;;
        49 | 5[0-7]) echo "digit $((_ui_b - 48))" ;;
        *) echo none ;;
    esac
}

# The Nth "value|label|hint" item's field F (1 value, 2 label, 3 hint).
_ui_item() {
    _ui_n=$1
    _ui_f=$2
    shift 2
    _ui_it=''
    eval "_ui_it=\${$_ui_n}"
    case "$_ui_f" in
        1) printf '%s' "${_ui_it%%|*}" ;;
        2)
            _ui_rest=${_ui_it#*|}
            printf '%s' "${_ui_rest%%|*}"
            ;;
        3)
            _ui_rest=${_ui_it#*|}
            case "$_ui_rest" in *'|'*) printf '%s' "${_ui_rest#*|}" ;; esac
            ;;
    esac
}

# ui_select VAR TITLE DEFAULT "value|label|hint"...: keeps a value already set.
ui_select() {
    _ui_var=$1
    _ui_title=$2
    _ui_default=$3
    shift 3
    eval "_ui_current=\${$_ui_var:-}"
    [ -z "$_ui_current" ] || return 0
    if [ "$UI_MODE" = none ]; then
        eval "$_ui_var=\$_ui_default"
        return 0
    fi
    _ui_count=$#
    _ui_cur=1
    _ui_i=1
    while [ $_ui_i -le $_ui_count ]; do
        [ "$(_ui_item $_ui_i 1 "$@")" != "$_ui_default" ] || _ui_cur=$_ui_i
        _ui_i=$((_ui_i + 1))
    done
    if [ "$UI_MODE" = rich ]; then
        _ui_select_rich "$@"
    else
        _ui_select_plain "$@"
    fi
    _ui_value=$(_ui_item $_ui_cur 1 "$@")
    eval "$_ui_var=\$_ui_value"
}

_ui_select_plain() {
    printf '%s %s\n' "$(_ui_sym ask)" "$_ui_title" >/dev/tty
    _ui_i=1
    while [ $_ui_i -le $_ui_count ]; do
        _ui_mark=' '
        [ $_ui_i != $_ui_cur ] || _ui_mark='*'
        _ui_hint=$(_ui_item $_ui_i 3 "$@")
        [ -z "$_ui_hint" ] || _ui_hint="  ($_ui_hint)"
        printf '  %s %d) %s%s\n' "$_ui_mark" $_ui_i "$(_ui_item $_ui_i 2 "$@")" "$_ui_hint" >/dev/tty
        _ui_i=$((_ui_i + 1))
    done
    while :; do
        t ui.choose "$_ui_cur" >/dev/tty
        read -r _ui_answer </dev/tty || _ui_answer=''
        [ -n "$_ui_answer" ] || return 0
        _ui_i=1
        while [ $_ui_i -le $_ui_count ]; do
            if [ "$_ui_answer" = $_ui_i ] || [ "$_ui_answer" = "$(_ui_item $_ui_i 1 "$@")" ]; then
                _ui_cur=$_ui_i
                return 0
            fi
            _ui_i=$((_ui_i + 1))
        done
        t ui.pick >/dev/tty
        printf '\n' >/dev/tty
    done
}

_ui_select_rich() {
    printf '%s %s\n' "$(_ui_c '32;1' "$(_ui_sym ask)")" "$(_ui_c 1 "$_ui_title")" >/dev/tty
    # No line wrap while drawing, so each item takes exactly one line and the
    # redraw moves back the right number of lines.
    printf '\033[?25l\033[?7l' >/dev/tty
    _ui_drawn=1
    stty -icanon -echo min 1 time 0 </dev/tty
    while :; do
        _ui_i=1
        while [ $_ui_i -le $_ui_count ]; do
            _ui_hint=$(_ui_item $_ui_i 3 "$@")
            [ -z "$_ui_hint" ] || _ui_hint="  $(_ui_c 2 "$_ui_hint")"
            if [ $_ui_i = $_ui_cur ]; then
                printf '\r\033[K  %s %s%s\n' "$(_ui_c 36 "$(_ui_sym pointer)")" \
                    "$(_ui_c '36;1' "$(_ui_item $_ui_i 2 "$@")")" "$_ui_hint" >/dev/tty
            else
                printf '\r\033[K    %s%s\n' "$(_ui_item $_ui_i 2 "$@")" "$_ui_hint" >/dev/tty
            fi
            _ui_i=$((_ui_i + 1))
        done
        printf '\r\033[K  %s' "$(_ui_c 2 "$(t ui.keys)")" >/dev/tty
        _ui_k=$(_ui_read_key)
        case "$_ui_k" in
            up) if [ $_ui_cur -gt 1 ]; then _ui_cur=$((_ui_cur - 1)); else _ui_cur=$_ui_count; fi ;;
            down) if [ $_ui_cur -lt $_ui_count ]; then _ui_cur=$((_ui_cur + 1)); else _ui_cur=1; fi ;;
            digit*)
                _ui_d=${_ui_k#digit }
                [ "$_ui_d" -lt 1 ] || [ "$_ui_d" -gt $_ui_count ] || _ui_cur=$_ui_d
                ;;
            quit) _ui_cleanup; exit 130 ;;
        esac
        # Back to the first item.
        printf '\r\033[%dA' $_ui_count >/dev/tty
        [ "$_ui_k" != enter ] || break
    done
    stty "$_ui_stty" </dev/tty
    # Fold the menu into one answered line.
    printf '\033[1A\r\033[J%s %s %s %s\n' "$(_ui_c 32 "$(_ui_sym ok)")" "$_ui_title" "$(_ui_c 2 '·')" \
        "$(_ui_c 36 "$(_ui_item $_ui_cur 2 "$@")")" >/dev/tty
    printf '\033[?25h\033[?7h' >/dev/tty
    _ui_drawn=0
}

# ui_input VAR PROMPT DEFAULT [CHECK]: CHECK VALUE prints why it is wrong and
# fails. Keeps a value already set (still checked).
ui_input() {
    _ui_var=$1
    _ui_prompt=$2
    _ui_default=$3
    _ui_check=${4:-}
    eval "_ui_answer=\${$_ui_var:-}"
    if [ -n "$_ui_answer" ] || [ "$UI_MODE" = none ]; then
        [ -n "$_ui_answer" ] || _ui_answer=$_ui_default
        if [ -n "$_ui_check" ] && [ -n "$_ui_answer" ]; then
            _ui_why=$("$_ui_check" "$_ui_answer") || die "$_ui_why"
        fi
        eval "$_ui_var=\$_ui_answer"
        return 0
    fi
    while :; do
        if [ -n "$_ui_default" ]; then
            printf '%s %s %s ' "$(_ui_c '32;1' "$(_ui_sym ask)")" "$(_ui_c 1 "$_ui_prompt")" \
                "$(_ui_c 2 "[$_ui_default]")" >/dev/tty
        else
            printf '%s %s ' "$(_ui_c '32;1' "$(_ui_sym ask)")" "$(_ui_c 1 "$_ui_prompt")" >/dev/tty
        fi
        read -r _ui_answer </dev/tty || _ui_answer=''
        [ -n "$_ui_answer" ] || _ui_answer=$_ui_default
        if [ -n "$_ui_check" ] && [ -n "$_ui_answer" ]; then
            if ! _ui_why=$("$_ui_check" "$_ui_answer"); then
                printf '  %s %s\n' "$(_ui_c 33 "$(_ui_sym warn)")" "$_ui_why" >/dev/tty
                continue
            fi
        fi
        break
    done
    eval "$_ui_var=\$_ui_answer"
}

# ui_secret VAR PROMPT MIN: hidden, twice. Keeps a value already set.
ui_secret() {
    eval "_ui_answer=\${$1:-}"
    [ -z "$_ui_answer" ] || return 0
    [ "$UI_MODE" != none ] || return 0
    while :; do
        printf '%s %s ' "$(_ui_c '32;1' "$(_ui_sym ask)")" "$(_ui_c 1 "$2")" >/dev/tty
        stty -echo </dev/tty
        read -r _ui_first </dev/tty || _ui_first=''
        printf '\n%s %s ' "$(_ui_c '32;1' "$(_ui_sym ask)")" "$(_ui_c 1 "$(t ui.again)")" >/dev/tty
        read -r _ui_second </dev/tty || _ui_second=''
        stty echo </dev/tty
        printf '\n' >/dev/tty
        if [ "$_ui_first" != "$_ui_second" ]; then
            printf '  %s %s\n' "$(_ui_c 33 "$(_ui_sym warn)")" "$(t ui.differ)" >/dev/tty
        elif [ ${#_ui_first} -lt "$3" ]; then
            printf '  %s %s\n' "$(_ui_c 33 "$(_ui_sym warn)")" "$(t ui.short "$3")" >/dev/tty
        else
            break
        fi
    done
    eval "$1=\$_ui_first"
}

# ui_confirm TITLE DEFAULT(yes|no): succeeds on yes. Unattended: the default.
ui_confirm() {
    _ui_yn=''
    ui_select _ui_yn "$1" "$2" "yes|$(t ui.yes)" "no|$(t ui.no)"
    [ "$_ui_yn" = yes ]
}

# --- Versions -------------------------------------------------------------------

# ui_version_lt A B: whether version A is older than B (numbers only).
ui_version_lt() {
    _ui_a=${1#v}
    _ui_b=${2#v}
    while [ -n "$_ui_a$_ui_b" ]; do
        _ui_x=${_ui_a%%.*}
        _ui_y=${_ui_b%%.*}
        case "$_ui_a" in *.*) _ui_a=${_ui_a#*.} ;; *) _ui_a='' ;; esac
        case "$_ui_b" in *.*) _ui_b=${_ui_b#*.} ;; *) _ui_b='' ;; esac
        _ui_x=$(printf '%s' "$_ui_x" | tr -cd 0-9)
        _ui_y=$(printf '%s' "$_ui_y" | tr -cd 0-9)
        _ui_x=${_ui_x:-0}
        _ui_y=${_ui_y:-0}
        [ "$_ui_x" -eq "$_ui_y" ] || { [ "$_ui_x" -lt "$_ui_y" ]; return; }
    done
    return 1
}

# --- Listen addresses -----------------------------------------------------------

# Processes and the docker-proxy port that count as this install's own.
UI_OURS=''
UI_OURS_PORT=''

# The program on ADDRESS:PORT that is not ours, or empty. Addresses clash
# when equal or when either is a wildcard. Prints ? for a program it cannot
# name, and ! when there is no ss or netstat to tell.
ui_port_owner() {
    _ui_addr=$1
    _ui_port=$2
    if _ui_has ss; then
        _ui_list=$(ss -ltnp 2>/dev/null | awk '$1 != "State" { print $4, $6 }')
    elif _ui_has netstat; then
        _ui_list=$(netstat -ltnp 2>/dev/null | awk '$1 ~ /^tcp/ { print $4, $7 }')
    else
        echo '!'
        return 0
    fi
    printf '%s\n' "$_ui_list" | awk -v addr="$_ui_addr" -v port="$_ui_port" \
        -v ours=" $UI_OURS " -v ours_port="$UI_OURS_PORT" '
        function wild(a) { return a == "0.0.0.0" || a == "*" || a == "[::]" || a == "::" || a == "" }
        {
            local = $1
            p = local; sub(/.*:/, "", p)
            a = local; sub(/:[^:]*$/, "", a)
            if (p != port) next
            if (!(a == addr || wild(a) || wild(addr))) next
            name = $2
            if (match(name, /\("[^"]*"/)) name = substr(name, RSTART + 2, RLENGTH - 3)
            else { sub(/^[0-9]*\//, "", name) }
            if (name == "" || name == "-" || name ~ /:/) name = "?"
            if (index(ours, " " name " ")) next
            # Docker publishes the port of this install through docker-proxy, which
            # may also show unnamed (another PID namespace).
            if ((name == "docker-proxy" || name == "?") && port == ours_port) next
            print name
            exit
        }'
}

_ui_check_port() {
    case "$1" in
        '' | *[!0-9]*) t ui.port.bad; return 1 ;;
    esac
    if [ "$1" -lt 1 ] || [ "$1" -gt 65535 ]; then
        t ui.port.bad
        return 1
    fi
    _ui_owner=$(ui_port_owner "$_ui_listen_addr" "$1")
    case "$_ui_owner" in
        '') ;;
        '!') printf '  %s\n' "$(t ui.port.unknown)" >/dev/tty 2>/dev/null || true ;;
        *) t ui.port.used "$1" "$_ui_listen_addr" "$(ui_owner_name "$_ui_owner")"; return 1 ;;
    esac
}

# The machine's addresses, one "name address" per line.
_ui_addresses() {
    _ui_has ip || return 0
    ip -o addr show 2>/dev/null | awk '{
        split($4, a, "/")
        if ($3 == "inet") print $2, a[1]
        else if ($3 == "inet6" && a[1] !~ /^fe80/) print $2, "[" a[1] "]"
    }'
}

_ui_check_address() {
    case "$1" in
        0.0.0.0 | '[::]') return 0 ;;
    esac
    case "$1" in
        \[*\] | [0-9]*.[0-9]*.[0-9]*.[0-9]*) ;;
        *) t ui.listen.custom.bad "$1"; return 1 ;;
    esac
    if ! _ui_has ip; then
        printf '  %s\n' "$(t ui.listen.custom.unchecked "$1")" >/dev/tty 2>/dev/null || true
        return 0
    fi
    _ui_addresses | awk -v a="$1" '$2 == a { found = 1 } END { exit !found }' && return 0
    t ui.listen.custom.bad "$1"
    return 1
}

_ui_private() {
    case "$1" in
        10.* | 192.168.* | 172.1[6-9].* | 172.2[0-9].* | 172.3[01].*) return 0 ;;
        100.6[4-9].* | 100.[7-9][0-9].* | 100.1[01][0-9].* | 100.12[0-7].*) return 0 ;;
        \[f[cd]*) return 0 ;;
    esac
    return 1
}

# A port owner for people to read.
ui_owner_name() {
    case "$1" in
        '?') t ui.port.someone ;;
        *) printf '%s' "$1" ;;
    esac
}

# Docker's bridge address, which only this machine and its containers reach.
ui_bridge_address() {
    _ui_has ip || return 0
    ip -4 -o addr show docker0 2>/dev/null | awk '{ split($4, a, "/"); print a[1]; exit }'
}

# Whether SafeLine runs here; it refuses 127.0.0.1 as an upstream.
ui_safeline() {
    _ui_has docker && docker ps --format '{{.Names}}' 2>/dev/null | grep -q '^safeline-'
}

# ui_listen VAR DEFAULT_ADDRESS DEFAULT_PORT: ADDRESS:PORT, chosen from this
# machine's interfaces with the port checked. Keeps a value already set,
# which is still checked.
ui_listen() {
    _ui_lvar=$1
    eval "_ui_lvalue=\${$_ui_lvar:-}"
    if [ -n "$_ui_lvalue" ]; then
        case "$_ui_lvalue" in
            *:[0-9]*) ;;
            *) die "$(t ui.listen.bad "$_ui_lvalue")" ;;
        esac
        _ui_listen_addr=${_ui_lvalue%:*}
        _ui_why=$(_ui_check_port "${_ui_lvalue##*:}") || die "$_ui_why"
        return 0
    fi
    _ui_bridge=$(ui_bridge_address)
    _ui_default_addr=$2
    _ui_default_port=$3
    if [ -n "$_ui_bridge" ] && ui_safeline; then
        _ui_default_addr=$_ui_bridge
    fi
    set -- "127.0.0.1|$(t ui.listen.loopback)  127.0.0.1|$(t ui.listen.loopback.hint)"
    [ -z "$_ui_bridge" ] || set -- "$@" "$_ui_bridge|$(t ui.listen.bridge)  $_ui_bridge|$(t ui.listen.bridge.hint)"
    _ui_rows=$(_ui_addresses | awk '$1 != "lo" && $1 != "docker0" && $1 !~ /^(veth|br-[0-9a-f])/ && $2 !~ /^(127\.|\[::1\])/')
    _ui_old_ifs=$IFS
    IFS='
'
    for _ui_row in $_ui_rows; do
        _ui_name=${_ui_row%% *}
        _ui_address=${_ui_row#* }
        if [ -d "/sys/class/net/$_ui_name/bridge" ]; then
            _ui_kind=$(t ui.listen.bridged)
        elif _ui_private "$_ui_address"; then
            _ui_kind=$(t ui.listen.private)
        else
            _ui_kind=$(t ui.listen.public)
        fi
        set -- "$@" "$_ui_address|$_ui_name  $_ui_address|$_ui_kind"
    done
    IFS=$_ui_old_ifs
    set -- "$@" "0.0.0.0|$(t ui.listen.all)  0.0.0.0|$(t ui.listen.all.hint)" "custom|$(t ui.listen.custom)"
    _ui_choice=''
    ui_select _ui_choice "$(t ui.listen.q)" "$_ui_default_addr" "$@"
    if [ "$_ui_choice" = custom ]; then
        _ui_choice=''
        ui_input _ui_choice "$(t ui.listen.custom.q)" '' _ui_check_address
    fi
    _ui_listen_addr=$_ui_choice
    case "$_ui_choice" in
        127.* | '[::1]' | "$_ui_bridge") ;;
        *)
            if [ "$_ui_choice" = 0.0.0.0 ] || [ "$_ui_choice" = '[::]' ] || ! _ui_private "$_ui_choice"; then
                ui_warn "$(t ui.listen.exposed "$_ui_choice")"
            fi
            ;;
    esac
    _ui_port=''
    ui_input _ui_port "$(t ui.port.q)" "$_ui_default_port" _ui_check_port
    eval "$_ui_lvar=\$_ui_choice:\$_ui_port"
}
# <<< ui.sh

msg_en() {
    case "$1" in
        title) _m='control plane' ;;
        need.root) _m='run as root, e.g. with sudo' ;;
        need.linux) _m='the control plane runs on Linux' ;;
        need.curl) _m='curl is required' ;;
        need.sha) _m='sha256sum is required' ;;
        need.tty) _m='no terminal to ask on; pass the options and --yes' ;;
        need.release) _m='run the copy published with a release' ;;
        need.docker) _m='Docker is not installed; see https://docs.docker.com/engine/install/' ;;
        need.compose) _m='docker compose is not available' ;;
        need.systemd) _m='systemd is required to install directly; use --method docker' ;;
        need.domain) _m='--domain is required with --expose caddy' ;;
        need.admin) _m='an administrator email is required (--admin-email)' ;;
        need.password) _m='set YTTT_ADMIN_PASSWORD for the administrator' ;;
        need.postgres) _m='install PostgreSQL yourself, or choose SQLite' ;;
        need.installed) _m='yttt-control is not installed here' ;;
        bad.option) _m='unknown option %s' ;;
        bad.value) _m='%s needs a value' ;;
        bad.method) _m='--method is docker or native' ;;
        bad.database) _m='--database is sqlite or postgres' ;;
        bad.expose) _m='--expose is caddy or service' ;;
        bad.action) _m='unknown action %s' ;;
        bad.arch) _m='unsupported architecture %s' ;;
        bad.download) _m='could not download %s' ;;
        bad.sums) _m='SHA256SUMS lists no %s' ;;
        bad.checksum) _m='%s does not match its checksum' ;;
        bad.empty) _m='This cannot be empty.' ;;
        method.q) _m='How should the control plane run?' ;;
        method.docker) _m='Docker (compose)' ;;
        method.docker.hint) _m='easiest to upgrade and remove' ;;
        method.native) _m='Directly on this server (systemd)' ;;
        method.native.hint) _m='least memory' ;;
        database.q) _m='Which database?' ;;
        database.sqlite) _m='SQLite' ;;
        database.sqlite.hint) _m='a file, nothing else to run: personal use, small servers' ;;
        database.postgres) _m='PostgreSQL' ;;
        database.postgres.hint) _m='for many accounts and nodes' ;;
        expose.q) _m='How do browsers reach the console?' ;;
        expose.caddy) _m='Caddy on ports 80 and 443' ;;
        expose.caddy.hint) _m='automatic HTTPS; needs a domain' ;;
        expose.service) _m='Only the service' ;;
        expose.service.hint) _m='behind nginx, SafeLine or another proxy you run' ;;
        expose.taken) _m='Ports 80 and 443 are in use by %s, so the console goes behind it.' ;;
        expose.taken.caddy) _m='ports 80 and 443 are in use by %s, which Caddy needs; use --expose service and put the console behind %s' ;;
        expose.unknown) _m='Cannot check whether ports 80 and 443 are free; Caddy needs them.' ;;
        domain.q) _m='Domain for the console (its DNS points at this server)' ;;
        domain.unresolved) _m='%s does not resolve yet; Caddy gets the certificate once it does.' ;;
        email.q) _m='Email for certificate notices' ;;
        colocate.q) _m='Also run a relay node on this machine?' ;;
        colocate.yes) _m='Yes, behind the same Caddy' ;;
        colocate.yes.hint) _m='with its own domain, e.g. relay.%s' ;;
        colocate.no) _m='No' ;;
        colocate.no.hint) _m='deploy nodes elsewhere from the console' ;;
        relay.q) _m='Domain for the relay node (its DNS points at this server)' ;;
        public.q) _m='Address browsers and nodes use, e.g. https://control.example.com' ;;
        proxy.q) _m='Does the proxy in front set X-Forwarded-For?' ;;
        proxy.yes) _m='Yes' ;;
        proxy.yes.hint) _m='nginx, Caddy, SafeLine, a load balancer' ;;
        proxy.no) _m='No' ;;
        proxy.no.hint) _m='browsers connect straight to it' ;;
        signup.q) _m='Can people sign up on their own?' ;;
        signup.open) _m='Yes' ;;
        signup.open.hint) _m='anyone can create an account' ;;
        signup.closed) _m='No' ;;
        signup.closed.hint) _m='administrators open accounts' ;;
        admin.q) _m='Administrator email' ;;
        password.q) _m='Administrator password' ;;
        step.containers) _m='Starting the containers' ;;
        step.binaries) _m='Installing yttt-control %s' ;;
        step.postgres) _m='Installing PostgreSQL' ;;
        step.caddy) _m='Installing Caddy %s' ;;
        step.service) _m='Starting the service' ;;
        step.wait) _m='Waiting for the control plane to answer' ;;
        step.admin) _m='Creating the administrator %s' ;;
        step.relay) _m='Deploying a relay node for %s on this machine' ;;
        step.relay.wait) _m='Waiting for https://%s' ;;
        bad.start) _m='the control plane did not start' ;;
        bad.relay.wait) _m='https://%s does not answer yet; once it does, deploy node-1 from the console' ;;
        bad.relay.signin) _m='could not sign in as %s to deploy the node' ;;
        bad.relay.token) _m='the control plane gave no enrollment token' ;;
        relay.stays) _m='The relay node on this machine stays; remove it with: sudo /usr/local/bin/yttt-relay-install --uninstall' ;;
        done.title) _m='The control plane is running: %s' ;;
        done.secrets) _m='Settings and secrets (private): %s' ;;
        done.caddy) _m='Open TCP 80 and 443 to this server; the certificate arrives within a minute.' ;;
        done.proxy) _m='Point your reverse proxy at http://%s and keep its X-Forwarded-For header.' ;;
        done.relay) _m='The relay node runs here as node-1 at https://%s; also open UDP 7842.' ;;
        done.signin) _m='Sign in as %s, then deploy a relay node from Nodes.' ;;
        done.manage) _m='Manage it later: sudo %s' ;;
        status.title) _m='yttt-control %s' ;;
        status.method) _m='Runs         %s' ;;
        status.database) _m='Database     %s' ;;
        status.reached) _m='Reached at   %s' ;;
        status.listen) _m='Listens on   %s' ;;
        status.health) _m='Health       %s' ;;
        status.relay) _m='Relay node   %s' ;;
        status.where) _m='Settings     %s' ;;
        status.ok) _m='answering' ;;
        status.down) _m='not answering' ;;
        status.caddy) _m='Caddy, %s' ;;
        status.service) _m='%s, behind a proxy' ;;
        menu.q) _m='What now?' ;;
        menu.upgrade) _m='Upgrade' ;;
        menu.upgrade.hint) _m='to the latest release, keeping settings and data' ;;
        menu.reconfigure) _m='Reconfigure' ;;
        menu.reconfigure.hint) _m='how it is reached, keeping secrets and data' ;;
        menu.uninstall) _m='Uninstall' ;;
        menu.uninstall.hint) _m='settings and data can stay' ;;
        menu.quit) _m='Quit' ;;
        upgrade.latest) _m='Looking up the latest release' ;;
        upgrade.nolatest) _m='could not find the latest release of %s' ;;
        upgrade.current) _m='%s is the latest release, and it is installed.' ;;
        upgrade.again) _m='Reinstall %s anyway?' ;;
        upgrade.older) _m='%s is older than the installed %s, whose database it cannot read; pass --force to install it anyway' ;;
        upgrade.forced) _m='Installing the older %s; its control plane may refuse to start on this database.' ;;
        upgrade.fetch) _m='Downloading the installer of %s' ;;
        upgrade.handover) _m='Handing over to the installer of %s' ;;
        upgrade.done) _m='Upgraded to %s' ;;
        uninstall.q) _m='Remove the control plane from this machine?' ;;
        uninstall.keep) _m='Settings, secrets and data stay in %s; --purge deletes them.' ;;
        uninstall.purge) _m='--purge also deletes the database and the secrets in %s, for good.' ;;
        uninstall.purge.q) _m='Delete the database and the secrets? This cannot be undone.' ;;
        uninstall.docker.purged) _m='Removed the containers, their data and %s' ;;
        uninstall.docker.kept) _m='Stopped and removed the containers; data and settings stay in %s (--purge deletes them)' ;;
        uninstall.native.purged) _m='Removed the control plane, its settings and its data' ;;
        uninstall.native.kept) _m='Removed the control plane; settings stay in %s and data in %s (--purge deletes them)' ;;
        cancelled) _m='Nothing changed.' ;;
        *) _m='' ;;
    esac
    printf '%s' "$_m"
}

msg_zh() {
    case "$1" in
        title) _m='控制面' ;;
        need.root) _m='需要 root 权限运行，例如用 sudo' ;;
        need.linux) _m='控制面只能运行在 Linux 上' ;;
        need.curl) _m='需要 curl' ;;
        need.sha) _m='需要 sha256sum' ;;
        need.tty) _m='没有可以提问的终端；请给出参数并加 --yes' ;;
        need.release) _m='请运行随版本发布的那份脚本' ;;
        need.docker) _m='没有安装 Docker；见 https://docs.docker.com/engine/install/' ;;
        need.compose) _m='没有 docker compose' ;;
        need.systemd) _m='直接安装需要 systemd；或改用 --method docker' ;;
        need.domain) _m='--expose caddy 需要 --domain' ;;
        need.admin) _m='需要管理员邮箱（--admin-email）' ;;
        need.password) _m='请用 YTTT_ADMIN_PASSWORD 设置管理员密码' ;;
        need.postgres) _m='请自行安装 PostgreSQL，或选择 SQLite' ;;
        need.installed) _m='这台机器上没有安装 yttt-control' ;;
        bad.option) _m='未知参数 %s' ;;
        bad.value) _m='%s 需要一个值' ;;
        bad.method) _m='--method 只能是 docker 或 native' ;;
        bad.database) _m='--database 只能是 sqlite 或 postgres' ;;
        bad.expose) _m='--expose 只能是 caddy 或 service' ;;
        bad.action) _m='未知操作 %s' ;;
        bad.arch) _m='不支持的架构 %s' ;;
        bad.download) _m='无法下载 %s' ;;
        bad.sums) _m='SHA256SUMS 中没有 %s' ;;
        bad.checksum) _m='%s 与校验和不符' ;;
        bad.empty) _m='不能为空。' ;;
        method.q) _m='控制面如何运行？' ;;
        method.docker) _m='Docker（compose）' ;;
        method.docker.hint) _m='升级和删除最省事' ;;
        method.native) _m='直接安装在这台服务器上（systemd）' ;;
        method.native.hint) _m='内存占用最少' ;;
        database.q) _m='使用哪种数据库？' ;;
        database.sqlite) _m='SQLite' ;;
        database.sqlite.hint) _m='一个文件，无需额外服务：个人使用、小机器' ;;
        database.postgres) _m='PostgreSQL' ;;
        database.postgres.hint) _m='账号和节点较多时' ;;
        expose.q) _m='浏览器如何访问控制台？' ;;
        expose.caddy) _m='Caddy 占用 80 与 443 端口' ;;
        expose.caddy.hint) _m='自动 HTTPS；需要域名' ;;
        expose.service) _m='只启动服务' ;;
        expose.service.hint) _m='放在你自己的 nginx、SafeLine 等代理后面' ;;
        expose.taken) _m='80 与 443 端口已被 %s 占用，控制台将放在它后面。' ;;
        expose.taken.caddy) _m='80 与 443 端口已被 %s 占用，Caddy 无法使用；请改用 --expose service，把控制台放在 %s 后面' ;;
        expose.unknown) _m='无法检查 80 与 443 端口是否空闲；Caddy 需要它们。' ;;
        domain.q) _m='控制台的域名（DNS 指向这台服务器）' ;;
        domain.unresolved) _m='%s 还无法解析；解析生效后 Caddy 会申请证书。' ;;
        email.q) _m='接收证书通知的邮箱' ;;
        colocate.q) _m='同时在这台机器上运行一个 relay 节点？' ;;
        colocate.yes) _m='是，放在同一个 Caddy 后面' ;;
        colocate.yes.hint) _m='使用单独的域名，例如 relay.%s' ;;
        colocate.no) _m='否' ;;
        colocate.no.hint) _m='之后在控制台里部署到别的机器' ;;
        relay.q) _m='relay 节点的域名（DNS 指向这台服务器）' ;;
        public.q) _m='浏览器与节点访问控制面的地址，例如 https://control.example.com' ;;
        proxy.q) _m='前面的代理会设置 X-Forwarded-For 吗？' ;;
        proxy.yes) _m='会' ;;
        proxy.yes.hint) _m='nginx、Caddy、SafeLine、负载均衡' ;;
        proxy.no) _m='不会' ;;
        proxy.no.hint) _m='浏览器直接连接服务' ;;
        signup.q) _m='是否允许自助注册？' ;;
        signup.open) _m='允许' ;;
        signup.open.hint) _m='任何人都能创建账号' ;;
        signup.closed) _m='不允许' ;;
        signup.closed.hint) _m='由管理员开户' ;;
        admin.q) _m='管理员邮箱' ;;
        password.q) _m='管理员密码' ;;
        step.containers) _m='启动容器' ;;
        step.binaries) _m='安装 yttt-control %s' ;;
        step.postgres) _m='安装 PostgreSQL' ;;
        step.caddy) _m='安装 Caddy %s' ;;
        step.service) _m='启动服务' ;;
        step.wait) _m='等待控制面响应' ;;
        step.admin) _m='创建管理员 %s' ;;
        step.relay) _m='在这台机器上为 %s 部署 relay 节点' ;;
        step.relay.wait) _m='等待 https://%s 可访问' ;;
        bad.start) _m='控制面没有启动' ;;
        bad.relay.wait) _m='https://%s 还无法访问；可访问后请在控制台部署 node-1' ;;
        bad.relay.signin) _m='无法以 %s 登录来部署节点' ;;
        bad.relay.token) _m='控制面没有给出登记令牌' ;;
        relay.stays) _m='这台机器上的 relay 节点保留；删除它：sudo /usr/local/bin/yttt-relay-install --uninstall' ;;
        done.title) _m='控制面已运行：%s' ;;
        done.secrets) _m='配置与秘密（仅 root 可读）：%s' ;;
        done.caddy) _m='放行这台服务器的 TCP 80 与 443；证书会在一分钟内签发。' ;;
        done.proxy) _m='让反向代理转发到 http://%s，并保留 X-Forwarded-For 请求头。' ;;
        done.relay) _m='relay 节点以 node-1 运行在 https://%s；同时放行 UDP 7842。' ;;
        done.signin) _m='用 %s 登录控制台，然后在“节点”里部署 relay 节点。' ;;
        done.manage) _m='之后管理：sudo %s' ;;
        status.title) _m='yttt-control %s' ;;
        status.method) _m='运行方式   %s' ;;
        status.database) _m='数据库     %s' ;;
        status.reached) _m='访问地址   %s' ;;
        status.listen) _m='监听       %s' ;;
        status.health) _m='健康检查   %s' ;;
        status.relay) _m='relay 节点 %s' ;;
        status.where) _m='配置       %s' ;;
        status.ok) _m='正常响应' ;;
        status.down) _m='没有响应' ;;
        status.caddy) _m='Caddy，%s' ;;
        status.service) _m='%s，在代理之后' ;;
        menu.q) _m='要做什么？' ;;
        menu.upgrade) _m='升级' ;;
        menu.upgrade.hint) _m='到最新版本，保留配置与数据' ;;
        menu.reconfigure) _m='重新配置' ;;
        menu.reconfigure.hint) _m='更改访问方式，保留秘密与数据' ;;
        menu.uninstall) _m='卸载' ;;
        menu.uninstall.hint) _m='可以保留配置与数据' ;;
        menu.quit) _m='退出' ;;
        upgrade.latest) _m='查询最新版本' ;;
        upgrade.nolatest) _m='找不到 %s 的最新版本' ;;
        upgrade.current) _m='%s 已是最新版本，且已安装。' ;;
        upgrade.again) _m='仍然重新安装 %s？' ;;
        upgrade.older) _m='%s 比已安装的 %s 旧，读不了新版本的数据库；确实要安装请加 --force' ;;
        upgrade.forced) _m='正在安装更旧的 %s；它的控制面可能无法在这个数据库上启动。' ;;
        upgrade.fetch) _m='下载 %s 的安装脚本' ;;
        upgrade.handover) _m='交给 %s 的安装脚本继续' ;;
        upgrade.done) _m='已升级到 %s' ;;
        uninstall.q) _m='从这台机器删除控制面？' ;;
        uninstall.keep) _m='配置、秘密与数据保留在 %s；加 --purge 才会删除。' ;;
        uninstall.purge) _m='--purge 会同时永久删除数据库与 %s 中的秘密。' ;;
        uninstall.purge.q) _m='删除数据库与秘密？此操作无法撤销。' ;;
        uninstall.docker.purged) _m='已删除容器、其数据与 %s' ;;
        uninstall.docker.kept) _m='已停止并删除容器；数据与配置保留在 %s（--purge 才会删除）' ;;
        uninstall.native.purged) _m='已删除控制面、配置与数据' ;;
        uninstall.native.kept) _m='已删除控制面；配置保留在 %s，数据保留在 %s（--purge 才会删除）' ;;
        cancelled) _m='没有做任何改动。' ;;
        *) _m='' ;;
    esac
    printf '%s' "$_m"
}

# --- Helpers ---------------------------------------------------------------

random_hex() {
    od -An -N32 -tx1 /dev/urandom | tr -d ' \n'
}

arch() {
    case "$(uname -m)" in
        x86_64 | amd64) echo x86_64 ;;
        aarch64 | arm64) echo aarch64 ;;
        *) die "$(t bad.arch "$(uname -m)")" ;;
    esac
}

# A value from an existing control.toml: key = "value".
toml_value() {
    [ -f "$2" ] || return 0
    awk -F'"' -v key="$1" '$1 ~ "^" key "[ ]*=" { print $2; exit }' "$2"
}

# check_sum NAME FILE SUMS: FILE matches NAME's line in SUMS.
check_sum() {
    expected=$(awk -v name="$1" '$2 == name { print $1 }' "$3")
    [ -n "$expected" ] || die "$(t bad.sums "$1")"
    actual=$(sha256sum "$2" | awk '{ print $1 }')
    [ "$expected" = "$actual" ] || die "$(t bad.checksum "$1")"
}

# fetch NAME DEST: a file of this release, checked against its SHA256SUMS.
fetch() {
    if [ -n "$ASSET_DIR" ]; then
        cp "$ASSET_DIR/$1" "$2"
        sums="$ASSET_DIR/SHA256SUMS"
    else
        base="https://github.com/$REPO/releases/download/v$VERSION"
        curl -fsSL "$base/$1" -o "$2" || die "$(t bad.download "$1")"
        if [ ! -f "$WORK/SHA256SUMS" ]; then
            curl -fsSL "$base/SHA256SUMS" -o "$WORK/SHA256SUMS" || die "$(t bad.download SHA256SUMS)"
        fi
        sums="$WORK/SHA256SUMS"
    fi
    check_sum "$1" "$2" "$sums"
}

has() {
    command -v "$1" >/dev/null 2>&1
}

# Questions need a terminal unless --yes answers them.
need_answers() {
    ui_interactive || [ "$YES" = 1 ] || die "$(t need.tty)"
}

not_empty() {
    [ -n "$1" ] || { t bad.empty; return 1; }
}

# --- Install state ---------------------------------------------------------

# What an earlier run chose, so later runs do the same.
state_file() {
    if [ "$METHOD" = docker ]; then
        echo "$DIR/install.env"
    else
        echo "$NATIVE_CONFIG_DIR/install.env"
    fi
}

save_state() {
    cat >"$(state_file)" <<EOF
METHOD=$METHOD
DATABASE=$DATABASE
EXPOSE=$EXPOSE
DOMAIN=$DOMAIN
EMAIL=$EMAIL
LISTEN=$LISTEN
PUBLIC_URL=$PUBLIC_URL
BEHIND_PROXY=$BEHIND_PROXY
RELAY_DOMAIN=$RELAY_DOMAIN
DIR=$DIR
INSTALLED_VERSION=$VERSION
EOF
    chmod 600 "$(state_file)"
    mkdir -p "$NATIVE_CONFIG_DIR"
    echo "DIR=$DIR" >"$LOCATION"
}

# Finds an earlier install and sets EXISTING to its state file: Docker in
# --dir when it was given, otherwise native first, then Docker where the
# last install said, or in the default directory. Not in a subshell: it
# moves DIR to the install it finds.
find_install() {
    EXISTING=''
    if [ "$DIR_GIVEN" = 1 ] && [ -f "$DIR/install.env" ]; then
        EXISTING=$DIR/install.env
        return
    fi
    if [ -f "$NATIVE_CONFIG_DIR/install.env" ]; then
        EXISTING=$NATIVE_CONFIG_DIR/install.env
        return
    fi
    if [ "$DIR_GIVEN" = 0 ] && [ -f "$LOCATION" ]; then
        recorded=$(awk -F= '$1 == "DIR" { sub(/^[^=]*=/, ""); print; exit }' "$LOCATION")
        if [ -n "$recorded" ] && [ -f "$recorded/install.env" ]; then
            DIR=$recorded
        fi
    fi
    if [ -f "$DIR/install.env" ]; then
        EXISTING=$DIR/install.env
    fi
}

load_state() {
    # Values from options win over the saved ones.
    saved_method=$(awk -F= '$1 == "METHOD" { print $2 }' "$1")
    for key in DATABASE EXPOSE DOMAIN EMAIL LISTEN PUBLIC_URL BEHIND_PROXY RELAY_DOMAIN DIR INSTALLED_VERSION; do
        # shellcheck disable=SC2034 # read through eval
        value=$(awk -F= -v key="$key" '$1 == key { sub(/^[^=]*=/, ""); print; exit }' "$1")
        eval "SAVED_$key=\$value"
        eval "current=\${$key:-}"
        [ -n "$current" ] || eval "$key=\$value"
    done
    METHOD=${METHOD:-$saved_method}
}

# --- Configuration ---------------------------------------------------------

write_config() {
    path=$1
    database_url=$2
    # Reuse secrets from an existing file: never generate new ones over them.
    ticket_secret=$(toml_value ticket_secret "$path")
    secrets_key=$(toml_value secrets_key "$path")
    [ -n "$ticket_secret" ] || ticket_secret=$(random_hex)
    [ -n "$secrets_key" ] || secrets_key=$(random_hex)
    if [ "$EXPOSE" = caddy ]; then
        listen='127.0.0.1:8080'
        public_url="https://$DOMAIN"
        trusted=true
    else
        listen=$LISTEN
        public_url=$PUBLIC_URL
        if [ "$BEHIND_PROXY" = yes ]; then trusted=true; else trusted=false; fi
    fi
    # The container listens on every interface inside its own network.
    [ "$METHOD" = docker ] && listen='0.0.0.0:8080'
    signup=true
    [ "$SIGNUP" = closed ] && signup=false
    umask 077
    cat >"$path" <<EOF
# The yttt control plane, written by install-control.sh. Keep it private: it
# holds the ticket signing key and the key that seals console secrets.
listen = "$listen"
public_url = "$public_url"
trusted_proxy = $trusted
web_root = "$WEB_ROOT"
relay_binaries_dir = "$BIN_ROOT"
database_url = "$database_url"
# Changing it invalidates every relay ticket and needs every node redeployed.
ticket_secret = "$ticket_secret"
# Seals the GitHub and Google client secrets set up in the console.
secrets_key = "$secrets_key"

# Seeds for an empty database only; afterwards plans, sign-up and nodes are
# managed in the console.
[signup]
enabled = $signup
plan = "free"
node = "node-1"

[[plans]]
id = "free"
rate_bps = 250_000              # bytes per second (2 Mbps)
burst_bytes = 250_000
monthly_bytes = 10_737_418_240  # 10 GiB
throttled_rate_bps = 16_000     # after the quota (128 Kbps)
max_devices = 5
EOF
    umask 022
}

write_caddyfile() {
    path=$1
    upstream=$2
    {
        echo "# Written by install-control.sh: certificates for the console's domain."
        echo "{"
        echo "    email $EMAIL"
        if [ -n "${YTTT_CADDY_TEST:-}" ]; then
            set -- $YTTT_CADDY_TEST
            echo "    local_certs"
            echo "    https_port $1"
            echo "    http_port $2"
        else
            echo "    cert_issuer acme"
        fi
        echo "}"
        echo ""
        echo "$DOMAIN {"
        echo "    encode gzip"
        echo "    reverse_proxy $upstream"
        echo "}"
        if [ -n "$RELAY_DOMAIN" ]; then
            echo ""
            echo "# The relay node on this machine, which listens in the clear behind Caddy."
            echo "$RELAY_DOMAIN {"
            echo "    reverse_proxy 127.0.0.1:3340"
            echo "}"
            echo "# Devices probe for captive portals over plain HTTP."
            echo "http://$RELAY_DOMAIN {"
            echo "    handle /generate_204 {"
            echo "        reverse_proxy 127.0.0.1:3340"
            echo "    }"
            echo "    handle {"
            echo "        redir https://{host}{uri} permanent"
            echo "    }"
            echo "}"
        fi
    } >"$path"
}

# --- Questions for a new install ---------------------------------------------

ask_everything() {
    ui_select METHOD "$(t method.q)" docker \
        "docker|$(t method.docker)|$(t method.docker.hint)" \
        "native|$(t method.native)|$(t method.native.hint)"
    ui_select DATABASE "$(t database.q)" sqlite \
        "sqlite|$(t database.sqlite)|$(t database.sqlite.hint)" \
        "postgres|$(t database.postgres)|$(t database.postgres.hint)"
    # Caddy needs 80 and 443; when another proxy has them, the console goes
    # behind it. This install's own Caddy does not count.
    web_owner=$(ui_port_owner 0.0.0.0 443)
    [ -n "$web_owner" ] || web_owner=$(ui_port_owner 0.0.0.0 80)
    case "$web_owner" in
        '') ;;
        '!')
            ui_warn "$(t expose.unknown)"
            web_owner=''
            ;;
        *) web_owner=$(ui_owner_name "$web_owner") ;;
    esac
    if [ -n "$web_owner" ]; then
        [ "$EXPOSE" != caddy ] || die "$(t expose.taken.caddy "$web_owner" "$web_owner")"
        ui_step "$(t expose.taken "$web_owner")"
        EXPOSE=service
    else
        ui_select EXPOSE "$(t expose.q)" "${SAVED_EXPOSE:-caddy}" \
            "caddy|$(t expose.caddy)|$(t expose.caddy.hint)" \
            "service|$(t expose.service)|$(t expose.service.hint)"
    fi
    if [ "$EXPOSE" = caddy ]; then
        ui_interactive || [ -n "$DOMAIN" ] || die "$(t need.domain)"
        ui_input DOMAIN "$(t domain.q)" "${SAVED_DOMAIN:-}" not_empty
        ui_input EMAIL "$(t email.q)" "${SAVED_EMAIL:-admin@$DOMAIN}"
        if has getent && ! getent hosts "$DOMAIN" >/dev/null; then
            ui_warn "$(t domain.unresolved "$DOMAIN")"
        fi
        if [ "$NO_RELAY" = 0 ] && [ -z "$RELAY_DOMAIN" ]; then
            ui_select COLOCATE "$(t colocate.q)" yes \
                "yes|$(t colocate.yes)|$(t colocate.yes.hint "$DOMAIN")" \
                "no|$(t colocate.no)|$(t colocate.no.hint)"
            if [ "$COLOCATE" = yes ]; then
                ui_input RELAY_DOMAIN "$(t relay.q)" "relay.$DOMAIN" not_empty
            fi
        fi
    else
        saved=${SAVED_LISTEN:-127.0.0.1:8080}
        ui_listen LISTEN "${saved%:*}" "${saved##*:}"
        ui_input PUBLIC_URL "$(t public.q)" \
            "${SAVED_PUBLIC_URL:-http://$(hostname -f 2>/dev/null || hostname):${LISTEN##*:}}" not_empty
        ui_select BEHIND_PROXY "$(t proxy.q)" "${SAVED_BEHIND_PROXY:-yes}" \
            "yes|$(t proxy.yes)|$(t proxy.yes.hint)" \
            "no|$(t proxy.no)|$(t proxy.no.hint)"
    fi
    ui_select SIGNUP "$(t signup.q)" open \
        "open|$(t signup.open)|$(t signup.open.hint)" \
        "closed|$(t signup.closed)|$(t signup.closed.hint)"
    ui_interactive || [ -n "$ADMIN_EMAIL" ] || die "$(t need.admin)"
    ui_input ADMIN_EMAIL "$(t admin.q)" '' not_empty
    ui_secret ADMIN_PASSWORD "$(t password.q)" 8
    [ -n "$ADMIN_PASSWORD" ] || die "$(t need.password)"
}

# --- Docker ------------------------------------------------------------------

docker_install() {
    has docker || die "$(t need.docker)"
    docker compose version >/dev/null 2>&1 || die "$(t need.compose)"
    mkdir -p "$DIR/data"
    image=${IMAGE:-ghcr.io/yuworm/yttt-relay:$VERSION}
    WEB_ROOT=/usr/share/yttt-control/web
    BIN_ROOT=/usr/share/yttt-control/bin
    postgres_password=$(awk -F= '$1 == "POSTGRES_PASSWORD" { print $2 }' "$DIR/.env" 2>/dev/null || true)
    [ -n "$postgres_password" ] || postgres_password=$(random_hex)
    if [ "$DATABASE" = postgres ]; then
        database_url="postgres://postgres:$postgres_password@postgres:5432/postgres"
    else
        database_url="sqlite:///var/lib/yttt-control/control.db"
    fi
    write_config "$DIR/control.toml" "$database_url"
    umask 077
    {
        echo "YTTT_IMAGE=$image"
        echo "POSTGRES_PASSWORD=$postgres_password"
    } >"$DIR/.env"
    umask 022
    if [ "$EXPOSE" = caddy ]; then
        published='127.0.0.1:8080:8080'
        write_caddyfile "$DIR/Caddyfile" 127.0.0.1:8080
    else
        published="${LISTEN%:*}:${LISTEN##*:}:8080"
    fi
    {
        echo "# Written by install-control.sh; rerun it to change the setup."
        echo "services:"
        echo "  control:"
        echo "    image: \${YTTT_IMAGE}"
        echo "    restart: unless-stopped"
        echo "    volumes:"
        echo "      - ./control.toml:/etc/yttt/control.toml:ro"
        echo "      - ./data:/var/lib/yttt-control"
        echo "    ports:"
        echo "      - \"$published\""
        if [ "$DATABASE" = postgres ]; then
            echo "    depends_on:"
            echo "      postgres:"
            echo "        condition: service_healthy"
            echo "  postgres:"
            echo "    image: postgres:17-alpine"
            echo "    restart: unless-stopped"
            echo "    environment:"
            echo "      POSTGRES_PASSWORD: \${POSTGRES_PASSWORD}"
            echo "    volumes:"
            echo "      - pgdata:/var/lib/postgresql/data"
            echo "    healthcheck:"
            echo "      test: [\"CMD\", \"pg_isready\", \"-U\", \"postgres\"]"
            echo "      interval: 2s"
            echo "      retries: 30"
        fi
        if [ "$EXPOSE" = caddy ]; then
            echo "  caddy:"
            echo "    image: caddy:$CADDY_VERSION"
            echo "    restart: unless-stopped"
            # Host networking: it reaches the control plane on 127.0.0.1.
            echo "    network_mode: host"
            echo "    volumes:"
            echo "      - ./Caddyfile:/etc/caddy/Caddyfile:ro"
            echo "      - caddy_data:/data"
            echo "      - caddy_config:/config"
        fi
        if [ "$DATABASE" = postgres ] || [ "$EXPOSE" = caddy ]; then
            echo "volumes:"
            [ "$DATABASE" = postgres ] && echo "  pgdata:"
            if [ "$EXPOSE" = caddy ]; then
                echo "  caddy_data:"
                echo "  caddy_config:"
            fi
        fi
    } >"$DIR/compose.yml"
    # The image runs as nobody, which must read the config and own the data.
    chown "$IMAGE_UID:$IMAGE_UID" "$DIR/control.toml" "$DIR/data"
    chmod 600 "$DIR/control.toml"
    ui_spin "$(t step.containers)" compose_up || exit 1
    wait_answering "$DIR"
}

compose_up() {
    cd "$DIR"
    [ -n "$IMAGE" ] || docker compose pull -q
    docker compose up -d
}

docker_admin() {
    printf '%s\n' "$ADMIN_PASSWORD" | (cd "$DIR" && docker compose exec -T control \
        yttt-control create-admin --config /etc/yttt/control.toml "$ADMIN_EMAIL")
}

docker_upgrade() {
    image=${IMAGE:-ghcr.io/yuworm/yttt-relay:$VERSION}
    sed -i "s|^YTTT_IMAGE=.*|YTTT_IMAGE=$image|" "$DIR/.env"
    ui_spin "$(t step.containers)" compose_up || exit 1
    wait_answering "$DIR"
}

docker_uninstall() {
    remove_cert_sync
    if [ "$PURGE" = 1 ]; then
        (cd "$DIR" && docker compose down -v) || true
        rm -rf "$DIR"
        ui_ok "$(t uninstall.docker.purged "$DIR")"
    else
        (cd "$DIR" && docker compose down) || true
        rm -f "$DIR/install.env"
        ui_ok "$(t uninstall.docker.kept "$DIR")"
    fi
}

# --- Directly on the server ----------------------------------------------------

native_install() {
    has systemctl || die "$(t need.systemd)"
    WEB_ROOT=$NATIVE_SHARE/web
    BIN_ROOT=$NATIVE_SHARE/bin
    if ! id "$SERVICE_USER" >/dev/null 2>&1; then
        useradd --system --no-create-home --home-dir /nonexistent --shell /usr/sbin/nologin "$SERVICE_USER"
    fi
    ui_spin "$(t step.binaries "$VERSION")" native_binaries || exit 1
    mkdir -p "$NATIVE_CONFIG_DIR"
    if [ "$DATABASE" = postgres ]; then
        native_postgres
        database_url="postgres:///yttt_control?host=/var/run/postgresql&user=$SERVICE_USER"
    else
        database_url="sqlite://$NATIVE_DATA/control.db"
    fi
    write_config "$NATIVE_CONFIG" "$database_url"
    chown "root:$SERVICE_USER" "$NATIVE_CONFIG_DIR" "$NATIVE_CONFIG"
    chmod 750 "$NATIVE_CONFIG_DIR"
    chmod 640 "$NATIVE_CONFIG"
    after=network-online.target
    [ "$DATABASE" = postgres ] && after="$after postgresql.service"
    # An address such as Docker's bridge exists only once Docker is up.
    case "${LISTEN%:*}" in 127.0.0.1 | 0.0.0.0 | '[::]' | '[::1]' | '') ;; *) after="$after docker.service" ;; esac
    cat >"$NATIVE_UNIT" <<EOF
[Unit]
Description=yttt relay control plane
After=$after
Wants=network-online.target

[Service]
ExecStart=$NATIVE_BIN --config $NATIVE_CONFIG
User=$SERVICE_USER
Group=$SERVICE_USER
StateDirectory=yttt-control
AmbientCapabilities=CAP_NET_BIND_SERVICE
CapabilityBoundingSet=CAP_NET_BIND_SERVICE
NoNewPrivileges=yes
PrivateTmp=yes
ProtectSystem=strict
ProtectHome=yes
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
EOF
    systemctl daemon-reload
    systemctl enable yttt-control >/dev/null 2>&1
    systemctl restart yttt-control
    [ "$EXPOSE" = caddy ] && native_caddy
    wait_answering ""
}

native_binaries() {
    a=$(arch)
    fetch "yttt-control-$VERSION-linux-$a" "$WORK/yttt-control"
    install -m 755 "$WORK/yttt-control" "$NATIVE_BIN"
    fetch "yttt-control-web-$VERSION.tar.gz" "$WORK/web.tar.gz"
    rm -rf "$WORK/web" && mkdir -p "$WORK/web"
    tar -xzf "$WORK/web.tar.gz" -C "$WORK/web"
    mkdir -p "$NATIVE_SHARE"
    rm -rf "$NATIVE_SHARE/web.new" && mv "$WORK/web" "$NATIVE_SHARE/web.new"
    rm -rf "$NATIVE_SHARE/web" && mv "$NATIVE_SHARE/web.new" "$NATIVE_SHARE/web"
    # The relay binaries new nodes download from this control plane.
    mkdir -p "$NATIVE_SHARE/bin"
    for node_arch in x86_64 aarch64; do
        fetch "yttt-relay-$VERSION-linux-$node_arch" "$NATIVE_SHARE/bin/yttt-relay-linux-$node_arch"
        chmod 755 "$NATIVE_SHARE/bin/yttt-relay-linux-$node_arch"
    done
    (cd "$NATIVE_SHARE/bin" && sha256sum yttt-relay-linux-x86_64 yttt-relay-linux-aarch64 >SHA256SUMS)
}

install_postgres() {
    if has apt-get; then
        DEBIAN_FRONTEND=noninteractive apt-get install -y -q postgresql >/dev/null
    elif has dnf; then
        dnf install -y -q postgresql-server >/dev/null
        postgresql-setup --initdb >/dev/null 2>&1 || true
    else
        die "$(t need.postgres)"
    fi
}

native_postgres() {
    if ! has psql; then
        has apt-get || has dnf || die "$(t need.postgres)"
        ui_spin "$(t step.postgres)" install_postgres || exit 1
    fi
    systemctl enable --now postgresql >/dev/null 2>&1 || true
    # The service connects over the local socket as its own user (peer
    # authentication), so there is no database password to keep.
    exists=$(su postgres -s /bin/sh -c "psql -tAc \"SELECT 1 FROM pg_roles WHERE rolname = '$SERVICE_USER'\"")
    if [ "$exists" != 1 ]; then
        su postgres -s /bin/sh -c "psql -qc 'CREATE ROLE \"$SERVICE_USER\" LOGIN'"
    fi
    exists=$(su postgres -s /bin/sh -c "psql -tAc \"SELECT 1 FROM pg_database WHERE datname = 'yttt_control'\"")
    if [ "$exists" != 1 ]; then
        su postgres -s /bin/sh -c "createdb -O '$SERVICE_USER' yttt_control"
    fi
}

install_caddy() {
        case "$(arch)" in
            x86_64) caddy_arch=amd64 ;;
            aarch64) caddy_arch=arm64 ;;
        esac
        name="caddy_${CADDY_VERSION}_linux_$caddy_arch.tar.gz"
        base="https://github.com/caddyserver/caddy/releases/download/v$CADDY_VERSION"
        curl -fsSL "$base/$name" -o "$WORK/$name" || die "$(t bad.download "$name")"
        curl -fsSL "$base/caddy_${CADDY_VERSION}_checksums.txt" -o "$WORK/caddy.sums" || die "$(t bad.download "caddy_${CADDY_VERSION}_checksums.txt")"
        expected=$(awk -v name="$name" '$2 == name { print $1 }' "$WORK/caddy.sums")
        actual=$(sha512sum "$WORK/$name" | awk '{ print $1 }')
        [ -n "$expected" ] && [ "$expected" = "$actual" ] || die "$(t bad.checksum "$name")"
        tar -xzf "$WORK/$name" -C "$WORK" caddy
        install -m 755 "$WORK/caddy" /usr/local/bin/caddy
        touch "$NATIVE_CONFIG_DIR/caddy-installed"
}

native_caddy() {
    if ! has caddy; then
        ui_spin "$(t step.caddy "$CADDY_VERSION")" install_caddy || exit 1
    fi
    if ! id caddy >/dev/null 2>&1; then
        useradd --system --home-dir /var/lib/caddy --create-home --shell /usr/sbin/nologin caddy
    fi
    mkdir -p /etc/caddy
    write_caddyfile /etc/caddy/Caddyfile 127.0.0.1:8080
    cat >"$CADDY_UNIT" <<EOF
[Unit]
Description=Caddy, in front of the yttt control plane
After=network-online.target
Wants=network-online.target

[Service]
ExecStart=$(command -v caddy) run --environ --config /etc/caddy/Caddyfile
ExecReload=$(command -v caddy) reload --config /etc/caddy/Caddyfile --force
User=caddy
Group=caddy
Environment=HOME=/var/lib/caddy
AmbientCapabilities=CAP_NET_BIND_SERVICE
CapabilityBoundingSet=CAP_NET_BIND_SERVICE
NoNewPrivileges=yes
PrivateTmp=yes
ProtectSystem=full
LimitNOFILE=1048576
Restart=on-failure

[Install]
WantedBy=multi-user.target
EOF
    systemctl daemon-reload
    systemctl enable caddy >/dev/null 2>&1
    systemctl restart caddy
}

native_admin() {
    printf '%s\n' "$ADMIN_PASSWORD" | runuser -u "$SERVICE_USER" -- \
        "$NATIVE_BIN" create-admin --config "$NATIVE_CONFIG" "$ADMIN_EMAIL"
}

native_upgrade() {
    ui_spin "$(t step.binaries "$VERSION")" native_binaries || exit 1
    systemctl restart yttt-control
    wait_answering ""
}

native_uninstall() {
    remove_cert_sync
    systemctl disable --now yttt-control >/dev/null 2>&1 || true
    rm -f "$NATIVE_UNIT" "$NATIVE_BIN"
    rm -rf "$NATIVE_SHARE"
    if [ -f "$NATIVE_CONFIG_DIR/caddy-installed" ]; then
        systemctl disable --now caddy >/dev/null 2>&1 || true
        rm -f "$CADDY_UNIT" /usr/local/bin/caddy /etc/caddy/Caddyfile
    fi
    systemctl daemon-reload || true
    if [ "$PURGE" = 1 ]; then
        rm -rf "$NATIVE_CONFIG_DIR" "$NATIVE_DATA"
        if has psql && [ "$DATABASE" = postgres ]; then
            su postgres -s /bin/sh -c "dropdb --if-exists yttt_control" || true
            su postgres -s /bin/sh -c "psql -qc 'DROP ROLE IF EXISTS \"$SERVICE_USER\"'" || true
        fi
        userdel "$SERVICE_USER" 2>/dev/null || true
        ui_ok "$(t uninstall.native.purged)"
    else
        rm -f "$NATIVE_CONFIG_DIR/install.env"
        ui_ok "$(t uninstall.native.kept "$NATIVE_CONFIG_DIR" "$NATIVE_DATA")"
    fi
}

# --- A relay node on this machine ------------------------------------------------

# Deploys node-1 here, behind Caddy, with the administrator just created.
colocate_relay() {
    [ -n "$RELAY_DOMAIN" ] || return 0
    ui_step "$(t step.relay "$RELAY_DOMAIN")"
    # The node reaches the control plane at its public address, through Caddy.
    i=0
    until curl -fsS "https://$DOMAIN/healthz" >/dev/null 2>&1; do
        i=$((i + 1))
        [ $i -lt 120 ] || die "$(t bad.relay.wait "$DOMAIN")"
        sleep 2
    done
    local=$(local_target)
    password=$(printf '%s' "$ADMIN_PASSWORD" | sed 's/["\\]/\\&/g')
    printf '{"email": "%s", "password": "%s"}' "$ADMIN_EMAIL" "$password" >"$WORK/sign-in.json"
    cookie=$(curl -fsS -D - -o /dev/null -H 'x-yttt-web: 1' -H 'content-type: application/json' \
        --data-binary "@$WORK/sign-in.json" "http://$local/v1/web/session" \
        | tr -d '\r' | sed -n 's/^[Ss]et-[Cc]ookie: \([^;]*\).*/\1/p')
    rm -f "$WORK/sign-in.json"
    [ -n "$cookie" ] || die "$(t bad.relay.signin "$ADMIN_EMAIL")"
    deploy=$(curl -fsS -H "cookie: $cookie" -H 'x-yttt-web: 1' -H 'content-type: application/json' \
        -d "{\"domain\": \"$RELAY_DOMAIN\"}" "http://$local/v1/admin/nodes/node-1/enrollment")
    token=$(printf '%s' "$deploy" | sed -n "s/.*YTTT_ENROLL=.\([A-Za-z0-9_-]*\).*/\1/p")
    [ -n "$token" ] || die "$(t bad.relay.token)"
    curl -fsS "http://$local/install/relay.sh" -o "$WORK/relay.sh"
    # Caddy forwards to 127.0.0.1:3340, whatever runs beside it.
    YTTT_ENROLL=$token sh "$WORK/relay.sh" --method "$METHOD" --domain "$RELAY_DOMAIN" \
        --behind-proxy --listen 127.0.0.1:3340 --yes
    install_cert_sync
}

# Keeps /etc/yttt-relay/quic a readable copy of Caddy's certificate for the
# relay domain, which QUIC address discovery uses.
install_cert_sync() {
    if [ -n "${YTTT_CADDY_TEST:-}" ]; then issuer=local; else issuer=acme-v02.api.letsencrypt.org-directory; fi
    if [ "$METHOD" = docker ]; then
        volume=$(docker volume inspect -f '{{ .Mountpoint }}' "$(basename "$DIR")_caddy_data")
        store="$volume/caddy/certificates/$issuer/$RELAY_DOMAIN/$RELAY_DOMAIN"
        group=root
    else
        store="/var/lib/caddy/.local/share/caddy/certificates/$issuer/$RELAY_DOMAIN/$RELAY_DOMAIN"
        group=yttt-relay
    fi
    cat >"$CERT_SYNC" <<SYNC
#!/bin/sh
# Written by install-control.sh: copies Caddy's certificate for $RELAY_DOMAIN
# where the relay can read it. The key goes first; the relay reloads when
# the certificate changes.
set -eu
src='$store'
dst=/etc/yttt-relay/quic
[ -f "\$src.crt" ] && [ -f "\$src.key" ] || exit 0
cmp -s "\$src.crt" "\$dst/cert.pem" 2>/dev/null && exit 0
mkdir -p "\$dst"
install -m 640 -g $group "\$src.key" "\$dst/key.pem.new"
mv "\$dst/key.pem.new" "\$dst/key.pem"
install -m 644 "\$src.crt" "\$dst/cert.pem.new"
mv "\$dst/cert.pem.new" "\$dst/cert.pem"
SYNC
    chmod 755 "$CERT_SYNC"
    cat >"$CERT_UNIT.service" <<UNIT
[Unit]
Description=Copy the relay node's certificate from Caddy

[Service]
Type=oneshot
ExecStart=$CERT_SYNC
UNIT
    cat >"$CERT_UNIT.timer" <<UNIT
[Unit]
Description=Copy the relay node's certificate from Caddy after each renewal

[Timer]
OnBootSec=1min
OnUnitActiveSec=10min

[Install]
WantedBy=timers.target
UNIT
    systemctl daemon-reload
    systemctl enable --now yttt-relay-cert.timer >/dev/null
    "$CERT_SYNC"
}

remove_cert_sync() {
    [ -f "$CERT_UNIT.timer" ] || return 0
    systemctl disable --now yttt-relay-cert.timer >/dev/null 2>&1 || true
    rm -f "$CERT_UNIT.timer" "$CERT_UNIT.service" "$CERT_SYNC"
    systemctl daemon-reload || true
    ui_note "$(t relay.stays)"
}

# --- Shared ------------------------------------------------------------------

answering() {
    [ "$(curl -fsS -m 3 "http://$(local_target)/healthz" 2>/dev/null)" = ok ]
}

wait_until_answering() {
    i=0
    while [ $i -lt 60 ]; do
        answering && return 0
        i=$((i + 1))
        sleep 1
    done
    return 1
}

# Waits for the control plane to answer on its own port; $1 is the Docker
# directory, empty when installed directly.
wait_answering() {
    ui_spin "$(t step.wait)" wait_until_answering && return 0
    if [ -n "$1" ]; then
        (cd "$1" && docker compose logs --tail 20 control) >&2 || true
    else
        journalctl -u yttt-control -n 20 --no-pager >&2 || true
    fi
    die "$(t bad.start)"
}

# Where this machine reaches the control plane directly.
local_target() {
    if [ "$EXPOSE" = caddy ]; then
        echo 127.0.0.1:8080
    else
        host=${LISTEN%:*}
        case "$host" in 0.0.0.0 | '[::]' | '') host=127.0.0.1 ;; esac
        echo "$host:${LISTEN##*:}"
    fi
}

# Keeps this script as the management command. Run through a pipe there is
# no file to copy, so it downloads this release's.
install_manager() {
    if [ -f "$0" ] && grep -q '^# yttt-installer-protocol:' "$0" 2>/dev/null; then
        source=$0
    else
        source=$WORK/install-control.sh
        fetch install-control.sh "$source"
    fi
    cmp -s "$source" "$MANAGER" 2>/dev/null || install -m 755 "$source" "$MANAGER"
}

secrets_path() {
    if [ "$METHOD" = docker ]; then
        echo "$DIR/control.toml, $DIR/.env"
    else
        echo "$NATIVE_CONFIG"
    fi
}

summary() {
    if [ "$EXPOSE" = caddy ]; then
        url="https://$DOMAIN"
    else
        url=$PUBLIC_URL
    fi
    set -- "$(t done.secrets "$(secrets_path)")"
    if [ "$EXPOSE" = caddy ]; then
        set -- "$@" "$(t done.caddy)"
    elif [ "$BEHIND_PROXY" = yes ]; then
        set -- "$@" "$(t done.proxy "$LISTEN")"
    fi
    if [ -n "$RELAY_DOMAIN" ]; then
        set -- "$@" "$(t done.relay "$RELAY_DOMAIN")"
    elif [ "$ADMIN_EMAIL" != none ]; then
        set -- "$@" "$(t done.signin "$ADMIN_EMAIL")"
    fi
    ui_box "$(t done.title "$url")" "$@" "$(t done.manage "$MANAGER")"
}

status() {
    if [ "$METHOD" = docker ]; then how=$(t method.docker); else how=$(t method.native); fi
    if [ "$EXPOSE" = caddy ]; then
        reached=$(t status.caddy "https://$DOMAIN")
    else
        reached=$(t status.service "$PUBLIC_URL")
    fi
    if answering; then health=$(t status.ok); else health=$(t status.down); fi
    set -- "$(t status.method "$how")" "$(t status.database "$DATABASE")" \
        "$(t status.reached "$reached")" "$(t status.listen "$(local_target)")" \
        "$(t status.health "$health")"
    [ -z "$RELAY_DOMAIN" ] || set -- "$@" "$(t status.relay "https://$RELAY_DOMAIN")"
    if [ "$METHOD" = docker ]; then where=$DIR; else where=$NATIVE_CONFIG_DIR; fi
    ui_box "$(t status.title "${INSTALLED_VERSION:-?}")" "$@" "$(t status.where "$where")"
}

# --- Upgrades ------------------------------------------------------------------

# The latest release's version, from where GitHub redirects /releases/latest.
latest_version() {
    if [ -n "$ASSET_DIR" ]; then
        sed -n "s/^VERSION='\\([0-9.]*\\)'\$/\\1/p" "$ASSET_DIR/install-control.sh" | head -n 1
        return
    fi
    location=$(curl -fsSI -o /dev/null -w '%{redirect_url}' "https://github.com/$REPO/releases/latest" || true)
    tag=${location##*/}
    printf '%s\n' "$tag" | grep -Eq '^v[0-9]+\.[0-9]+\.[0-9]+$' || return 0
    echo "${tag#v}"
}

# Downloads the installer of release $1, checks it and hands over to it.
hand_over() {
    target=$1
    ui_step "$(t upgrade.fetch "$target")"
    handoff=$(mktemp)
    if [ -n "$ASSET_DIR" ]; then
        cp "$ASSET_DIR/install-control.sh" "$handoff"
        sums=$ASSET_DIR/SHA256SUMS
    else
        base="https://github.com/$REPO/releases/download/v$target"
        curl -fsSL "$base/install-control.sh" -o "$handoff" || die "$(t bad.download "$base/install-control.sh")"
        curl -fsSL "$base/SHA256SUMS" -o "$WORK/SHA256SUMS.$target" || die "$(t bad.download "$base/SHA256SUMS")"
        sums=$WORK/SHA256SUMS.$target
    fi
    check_sum install-control.sh "$handoff" "$sums"
    ui_step "$(t upgrade.handover "$target")"
    # Only options every release's installer knows; the language goes in
    # the environment.
    set -- --upgrade --dir "$DIR"
    [ "$YES" = 0 ] || set -- "$@" --yes
    [ -z "$ASSET_DIR" ] || set -- "$@" --asset-dir "$ASSET_DIR"
    [ -z "$IMAGE" ] || set -- "$@" --image "$IMAGE"
    ui_before_exec
    YTTT_HANDOFF_FILE=$handoff exec sh "$handoff" "$@"
}

upgrade() {
    installed=${INSTALLED_VERSION:-0.0.0}
    released=1
    case "$VERSION" in __*) released=0 ;; esac
    target=$UPGRADE_TO
    if [ -z "$target" ]; then
        if [ "$released" = 0 ] || ui_version_lt "$installed" "$VERSION"; then
            # This script is the newer release (or a test copy): upgrade here.
            target=$VERSION
        else
            ui_step "$(t upgrade.latest)"
            target=$(latest_version)
            [ -n "$target" ] || die "$(t upgrade.nolatest "$REPO")"
        fi
    fi
    if [ "$released" = 1 ] && ui_version_lt "$target" "$installed"; then
        [ "$FORCE" = 1 ] || die "$(t upgrade.older "$target" "$installed")"
        ui_warn "$(t upgrade.forced "$target")"
    fi
    if [ "$target" != "$VERSION" ]; then
        hand_over "$target"
    fi
    if [ "$released" = 1 ] && [ "$installed" = "$VERSION" ] && [ -z "$UPGRADE_TO" ]; then
        ui_ok "$(t upgrade.current "$VERSION")"
        ui_interactive || return 0
        ui_confirm "$(t upgrade.again "$VERSION")" no || return 0
    fi
    ui_detach
    "${METHOD}_upgrade"
    save_state
    install_manager
    ui_ok "$(t upgrade.done "$VERSION")"
}

# --- Menu ----------------------------------------------------------------------

uninstall() {
    if [ "$METHOD" = docker ]; then where=$DIR; else where="$NATIVE_CONFIG_DIR, $NATIVE_DATA"; fi
    if ui_interactive; then
        if [ "$PURGE" = 1 ]; then
            ui_warn "$(t uninstall.purge "$where")"
        else
            ui_note "$(t uninstall.keep "$where")"
        fi
        ui_confirm "$(t uninstall.q)" no || { ui_note "$(t cancelled)"; return 0; }
        if [ "$PURGE" = 1 ]; then
            ui_confirm "$(t uninstall.purge.q)" no || { ui_note "$(t cancelled)"; return 0; }
        fi
    fi
    ui_detach
    "${METHOD}_uninstall"
    rm -f "$MANAGER" "$LOCATION"
}

reconfigure() {
    # On a terminal, ask again how it is reached, except what the options
    # say; --yes keeps the saved answers. Method and database stay.
    if [ "$YES" = 0 ]; then
        for pair in $given; do
            eval "${pair%%=*}=\${pair#*=}"
        done
    fi
    need_answers
    SIGNUP=closed
    ADMIN_EMAIL=none
    ADMIN_PASSWORD=none
    ask_everything
    ui_detach
    "${METHOD}_install"
    save_state
    install_manager
    summary
}

manage() {
    status
    if ! ui_interactive; then
        [ "$YES" = 1 ] || return 0
        ACTION=upgrade
    fi
    ui_select ACTION "$(t menu.q)" upgrade \
        "upgrade|$(t menu.upgrade)|$(t menu.upgrade.hint)" \
        "reconfigure|$(t menu.reconfigure)|$(t menu.reconfigure.hint)" \
        "uninstall|$(t menu.uninstall)|$(t menu.uninstall.hint)" \
        "quit|$(t menu.quit)"
}

main() {
    METHOD=''
    DATABASE=''
    EXPOSE=''
    DOMAIN=''
    EMAIL=''
    LISTEN=''
    PUBLIC_URL=''
    BEHIND_PROXY=''
    SIGNUP=''
    RELAY_DOMAIN=''
    COLOCATE=''
    NO_RELAY=0
    ADMIN_EMAIL=''
    ADMIN_PASSWORD=${YTTT_ADMIN_PASSWORD:-}
    DIR=/opt/yttt-control
    DIR_GIVEN=0
    ASSET_DIR=''
    IMAGE=''
    YES=0
    PURGE=0
    FORCE=0
    ACTION=''
    UPGRADE_TO=''
    INSTALLED_VERSION=''
    SAVED_EXPOSE=''
    SAVED_DOMAIN=''
    SAVED_EMAIL=''
    SAVED_LISTEN=''
    SAVED_PUBLIC_URL=''
    SAVED_BEHIND_PROXY=''
    while [ $# -gt 0 ]; do
        case "$1" in
            --method | --database | --expose | --domain | --email | --listen | --public-url | \
                --behind-proxy | --admin-email | --signup | --dir | --asset-dir | --image | \
                --relay-domain | --lang | --version)
                [ $# -ge 2 ] || die "$(t bad.value "$1")"
                case "$1" in
                    --method) METHOD=$2 ;;
                    --database) DATABASE=$2 ;;
                    --expose) EXPOSE=$2 ;;
                    --domain) DOMAIN=$2 ;;
                    --email) EMAIL=$2 ;;
                    --listen) LISTEN=$2 ;;
                    --public-url) PUBLIC_URL=$2 ;;
                    --behind-proxy) BEHIND_PROXY=$2 ;;
                    --admin-email) ADMIN_EMAIL=$2 ;;
                    --signup) SIGNUP=$2 ;;
                    --dir) DIR=$2; DIR_GIVEN=1 ;;
                    --asset-dir) ASSET_DIR=$2 ;;
                    --image) IMAGE=$2 ;;
                    --relay-domain) RELAY_DOMAIN=$2 ;;
                    --lang) YTTT_LANG=$2 ;;
                    --version) UPGRADE_TO=${2#v}; ACTION=upgrade ;;
                esac
                shift 2
                ;;
            --yes) YES=1; shift ;;
            --no-relay) NO_RELAY=1; shift ;;
            --upgrade) ACTION=upgrade; shift ;;
            --reconfigure) ACTION=reconfigure; shift ;;
            --uninstall) ACTION=uninstall; shift ;;
            --purge) PURGE=1; shift ;;
            --force) FORCE=1; shift ;;
            -h | --help) awk 'NR > 1 && /^#/ { sub(/^# ?/, ""); print; next } NR > 1 { exit }' "$0"; exit 0 ;;
            *) die "$(t bad.option "$1")" ;;
        esac
    done
    # A handed-over installer removes its downloaded copy when it is done.
    [ -z "${YTTT_HANDOFF_FILE:-}" ] || ui_on_exit 'rm -f "$YTTT_HANDOFF_FILE"'
    ui_init "$YES"
    ui_choose_language
    # An unpublished copy has no release to download from.
    case "$VERSION" in
        __*) [ -n "$ASSET_DIR$IMAGE" ] || die "$(t need.release)" ;;
    esac
    [ "$(id -u)" = 0 ] || die "$(t need.root)"
    [ "$(uname -s)" = Linux ] || die "$(t need.linux)"
    has curl || die "$(t need.curl)"
    has sha256sum || die "$(t need.sha)"
    WORK=$(mktemp -d)
    ui_on_exit 'rm -rf "$WORK"'
    [ -n "${YTTT_HANDOFF_FILE:-}" ] || ui_banner "$(t title)" "$VERSION"

    # What the options said, before the saved choices fill the gaps.
    given="EXPOSE=$EXPOSE DOMAIN=$DOMAIN EMAIL=$EMAIL LISTEN=$LISTEN PUBLIC_URL=$PUBLIC_URL BEHIND_PROXY=$BEHIND_PROXY"
    find_install
    if [ -n "$EXISTING" ]; then
        load_state "$EXISTING"
        # Its own service, Caddy and published port are not in the way.
        UI_OURS='yttt-control caddy'
        UI_OURS_PORT=${SAVED_LISTEN##*:}
        [ "$EXPOSE" != caddy ] || UI_OURS_PORT=8080
        [ -n "$ACTION" ] || manage
        case "$ACTION" in
            upgrade) upgrade ;;
            uninstall) uninstall ;;
            reconfigure) reconfigure ;;
            quit | '') ;;
            *) die "$(t bad.action "$ACTION")" ;;
        esac
        return
    fi
    [ "$ACTION" = "" ] || [ "$ACTION" = reconfigure ] || die "$(t need.installed)"
    need_answers
    ask_everything
    case "$METHOD" in docker | native) ;; *) die "$(t bad.method)" ;; esac
    case "$DATABASE" in sqlite | postgres) ;; *) die "$(t bad.database)" ;; esac
    case "$EXPOSE" in caddy | service) ;; *) die "$(t bad.expose)" ;; esac
    ui_detach
    "${METHOD}_install"
    ui_spin "$(t step.admin "$ADMIN_EMAIL")" "${METHOD}_admin" || exit 1
    save_state
    install_manager
    [ "$EXPOSE" != caddy ] || colocate_relay
    summary
}

main "$@"
