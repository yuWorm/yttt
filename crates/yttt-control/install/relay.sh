#!/bin/sh
# Installs a yttt relay node and enrolls it with the control plane at
# __CONTROL_URL__.
#
# The deploy command checked this script against the hash it was generated
# with. The relay binary is checked against SHA256SUMS from the same server,
# which only catches a truncated download: the trust in both comes from
# HTTPS to the control plane.
#
#   YTTT_ENROLL=<token> sh relay.sh [--method native|docker] [--domain relay.example.com]
#                                   [--public-url URL] [--binary-url URL] [--image IMAGE]
#                                   [--port PORT] [--no-systemd] [--yes] [--lang en|zh]
#                                   [--behind-proxy [--listen ADDRESS:PORT]]
#   With --behind-proxy, a reverse proxy on this machine (Caddy, nginx,
#   SafeLine) serves --domain over HTTPS: the relay listens on 127.0.0.1:3340,
#   or --listen ADDRESS:PORT for a proxy that refuses loopback upstreams, and
#   QUIC uses the certificate in /etc/yttt-relay/quic. Without a domain the
#   node listens on --port (default 3341) and only relays.
#
#   /usr/local/bin/yttt-relay-install              status, and a menu on a terminal
#   /usr/local/bin/yttt-relay-install --upgrade    the control plane's current script and relay
#   /usr/local/bin/yttt-relay-install --listen ADDRESS:PORT   move a node behind a proxy
#   /usr/local/bin/yttt-relay-install --uninstall  stop and remove the node from this machine
#
# On a terminal it asks how to run the node (directly with systemd, or in
# Docker), for its domain and where to listen; options answer those, and
# --yes or no terminal takes the defaults: directly, without a domain.
#
# Upgrading fetches this script again from the control plane and hands over
# to it, over HTTPS only (--allow-http otherwise), and never to an older one
# (--force otherwise).
#
# The relay only forwards traffic that is end-to-end encrypted between a
# user's devices; nothing installed here can reach those devices.
#
# yttt-installer-protocol: 1

set -eu

VERSION='__VERSION__'
CONTROL='__CONTROL_URL__'
IMAGE_DEFAULT='__IMAGE__'
PREFIX=/usr/local/bin
CONFIG_DIR=/etc/yttt-relay
STATE_DIR=/var/lib/yttt-relay
UNIT=/etc/systemd/system/yttt-relay.service
CONTAINER=yttt-relay
# Written by install-control.sh for a node behind its own Caddy.
COLOCATED_SYNC=/usr/local/bin/yttt-relay-cert-sync
# For testing beside a real node: every path under one directory and a
# container of its own. Docker only.
if [ -n "${YTTT_RELAY_ROOT:-}" ]; then
    PREFIX=$YTTT_RELAY_ROOT/bin
    CONFIG_DIR=$YTTT_RELAY_ROOT/etc
    STATE_DIR=$YTTT_RELAY_ROOT/state
    CONTAINER=yttt-relay-test
    mkdir -p "$PREFIX"
fi
MANAGER=$PREFIX/yttt-relay-install
UI_NAME=yttt-relay-install

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
        title) _m='relay node' ;;
        need.root) _m='run as root, e.g. with sudo' ;;
        need.linux) _m='relay nodes run on Linux' ;;
        need.arch) _m='unsupported architecture %s' ;;
        need.enroll) _m='YTTT_ENROLL is not set; copy the deploy command from the console again' ;;
        need.curl) _m='curl is required' ;;
        need.sha) _m='sha256sum is required' ;;
        need.docker) _m='Docker is not installed; see https://docs.docker.com/engine/install/' ;;
        need.systemd) _m='systemd is required; rerun with --no-systemd to run in the foreground' ;;
        need.node) _m='this machine has no enrolled node' ;;
        bad.option) _m='unknown option %s' ;;
        bad.value) _m='%s needs a value' ;;
        bad.method) _m='--method is native or docker' ;;
        bad.proxy) _m='--behind-proxy needs the --domain the proxy serves' ;;
        bad.listen) _m='--listen is for a node behind a proxy (--behind-proxy)' ;;
        bad.port) _m='--port is for a node without a domain' ;;
        bad.resolve) _m='%s does not resolve; point its DNS at this machine first' ;;
        method.q) _m='How should the relay run?' ;;
        method.native) _m='Directly on this server' ;;
        method.native.hint) _m='a systemd service' ;;
        method.docker) _m='In Docker' ;;
        method.docker.hint) _m='host networking, the image of this control plane' ;;
        domain.about) _m='A domain gives the node HTTPS and lets devices find direct paths; without one it only relays.' ;;
        domain.q) _m='Domain pointing at this server (empty for none)' ;;
        ports.taken) _m='Ports 443 and 80 are in use by %s, so the node cannot serve %s itself.' ;;
        ports.taken.yes) _m='ports 443 and 80 are in use by %s; rerun with --behind-proxy [--listen ADDRESS:PORT] to run behind it, or without --domain' ;;
        ports.unknown) _m='Cannot check whether ports 443 and 80 are free; the node needs them.' ;;
        ports.q) _m='How should the node serve %s?' ;;
        ports.behind) _m='Behind %s' ;;
        ports.behind.hint) _m='it serves the domain over HTTPS and forwards to the node' ;;
        ports.nodomain) _m='Without a domain' ;;
        ports.nodomain.hint) _m='on its own port; it only relays, devices find no direct paths' ;;
        port.q) _m='Port for the node' ;;
        step.download) _m='Downloading the relay' ;;
        step.pull) _m='Pulling %s' ;;
        step.enroll) _m='Enrolling with %s' ;;
        step.start) _m='Starting the node' ;;
        step.wait) _m='Waiting for the relay to answer' ;;
        bad.checksum.list) _m='the control plane lists no checksum for %s' ;;
        bad.checksum) _m='the downloaded relay does not match its checksum' ;;
        bad.pull) _m='could not pull %s' ;;
        bad.download) _m='could not download %s' ;;
        warn.noanswer) _m='The relay does not answer yet; see: %s' ;;
        done.title) _m='The node is enrolled as %s' ;;
        done.proxy) _m='Point the proxy for %s at http://%s, with WebSocket allowed.' ;;
        done.quic) _m='Open UDP 7842 to this machine. For direct paths, put the certificate and key of %s in %s/quic/cert.pem and key.pem.' ;;
        done.domain) _m='Open TCP 443 and 80 and UDP 7842 to this machine; the certificate is requested in the background.' ;;
        done.plain) _m='Open TCP %s to this machine.' ;;
        done.online) _m='The console shows the node online within a minute.' ;;
        done.manage) _m='Manage it later: sudo %s' ;;
        foreground) _m='Running in the foreground (--no-systemd)' ;;
        status.title) _m='Relay node %s' ;;
        status.url) _m='Address      %s' ;;
        status.method) _m='Runs         %s' ;;
        status.listen) _m='Listens on   %s' ;;
        status.version) _m='Version      %s' ;;
        status.health) _m='Health       %s' ;;
        status.quic) _m='QUIC cert    %s' ;;
        status.control) _m='Control      %s' ;;
        status.ok) _m='answering' ;;
        status.down) _m='not answering' ;;
        status.present) _m='in place' ;;
        status.missing) _m='missing: devices only relay' ;;
        status.behind) _m='%s, behind a proxy' ;;
        menu.q) _m='What now?' ;;
        menu.upgrade) _m='Upgrade' ;;
        menu.upgrade.hint) _m='the relay and this script, from the control plane' ;;
        menu.listen) _m='Change the listen address' ;;
        menu.listen.hint) _m='where the proxy forwards to' ;;
        menu.uninstall) _m='Uninstall' ;;
        menu.uninstall.hint) _m='remove the node from this machine' ;;
        menu.quit) _m='Quit' ;;
        upgrade.http) _m='the control plane is plain http://, where anyone in between could hand this machine a script to run as root; upgrade over https://, or pass --allow-http' ;;
        upgrade.fetch) _m='Fetching the installer from the control plane' ;;
        upgrade.notours) _m='the control plane sent something that is not a yttt installer' ;;
        upgrade.older) _m='the control plane offers %s, older than this node (%s); upgrade the control plane first, or pass --force' ;;
        upgrade.handover) _m='Handing over to the installer of %s' ;;
        upgrade.done) _m='Upgraded to %s' ;;
        listen.colocated) _m='this node is behind the control plane'"'"'s Caddy, which forwards to 127.0.0.1:3340; its address stays' ;;
        listen.direct) _m='this node serves its own address; the listen address is only for a node behind a proxy' ;;
        listen.done) _m='The node listens on %s; point the proxy there.' ;;
        uninstall.q) _m='Remove the node from this machine? This deletes:' ;;
        uninstall.what) _m='%s, its settings in %s and its state in %s' ;;
        uninstall.done) _m='Removed; disable or delete the node in the console too' ;;
        cancelled) _m='Nothing changed.' ;;
        *) _m='' ;;
    esac
    printf '%s' "$_m"
}

msg_zh() {
    case "$1" in
        title) _m='relay 节点' ;;
        need.root) _m='需要 root 权限运行，例如用 sudo' ;;
        need.linux) _m='relay 节点只能运行在 Linux 上' ;;
        need.arch) _m='不支持的架构 %s' ;;
        need.enroll) _m='没有设置 YTTT_ENROLL；请从控制台重新复制部署命令' ;;
        need.curl) _m='需要 curl' ;;
        need.sha) _m='需要 sha256sum' ;;
        need.docker) _m='没有安装 Docker；见 https://docs.docker.com/engine/install/' ;;
        need.systemd) _m='需要 systemd；或加 --no-systemd 在前台运行' ;;
        need.node) _m='这台机器上没有已登记的节点' ;;
        bad.option) _m='未知参数 %s' ;;
        bad.value) _m='%s 需要一个值' ;;
        bad.method) _m='--method 只能是 native 或 docker' ;;
        bad.proxy) _m='--behind-proxy 需要同时给出代理所服务的 --domain' ;;
        bad.listen) _m='--listen 只用于代理之后的节点（--behind-proxy）' ;;
        bad.port) _m='--port 只用于没有域名的节点' ;;
        bad.resolve) _m='%s 无法解析；请先把它的 DNS 指向这台机器' ;;
        method.q) _m='relay 如何运行？' ;;
        method.native) _m='直接安装在这台服务器上' ;;
        method.native.hint) _m='systemd 服务' ;;
        method.docker) _m='在 Docker 中运行' ;;
        method.docker.hint) _m='host 网络，使用与控制面同版本的镜像' ;;
        domain.about) _m='有域名时节点提供 HTTPS，设备之间能找到直连路径；没有域名时只能中转。' ;;
        domain.q) _m='指向这台服务器的域名（留空表示不用域名）' ;;
        ports.taken) _m='443 与 80 端口已被 %s 占用，节点无法自己提供 %s。' ;;
        ports.taken.yes) _m='443 与 80 端口已被 %s 占用；加 --behind-proxy [--listen 地址:端口] 放在它后面，或去掉 --domain' ;;
        ports.unknown) _m='无法检查 443 与 80 端口是否空闲；节点需要它们。' ;;
        ports.q) _m='节点如何提供 %s？' ;;
        ports.behind) _m='放在 %s 后面' ;;
        ports.behind.hint) _m='由它提供 HTTPS 并转发给节点' ;;
        ports.nodomain) _m='不用域名' ;;
        ports.nodomain.hint) _m='使用独立端口；只能中转，设备之间无法直连' ;;
        port.q) _m='节点端口' ;;
        step.download) _m='下载 relay' ;;
        step.pull) _m='拉取镜像 %s' ;;
        step.enroll) _m='向 %s 登记' ;;
        step.start) _m='启动节点' ;;
        step.wait) _m='等待 relay 响应' ;;
        bad.checksum.list) _m='控制面没有提供 %s 的校验和' ;;
        bad.checksum) _m='下载的 relay 与校验和不符' ;;
        bad.pull) _m='无法拉取 %s' ;;
        bad.download) _m='无法下载 %s' ;;
        warn.noanswer) _m='relay 还没有响应；查看：%s' ;;
        done.title) _m='节点已登记为 %s' ;;
        done.proxy) _m='在代理里把 %s 转发到 http://%s，并允许 WebSocket。' ;;
        done.quic) _m='放行这台机器的 UDP 7842。要让设备直连，把 %s 的证书与私钥放到 %s/quic/cert.pem 与 key.pem。' ;;
        done.domain) _m='放行这台机器的 TCP 443、80 与 UDP 7842；证书在后台申请。' ;;
        done.plain) _m='放行这台机器的 TCP %s。' ;;
        done.online) _m='一分钟内控制台会显示节点在线。' ;;
        done.manage) _m='之后管理节点：sudo %s' ;;
        foreground) _m='在前台运行（--no-systemd）' ;;
        status.title) _m='relay 节点 %s' ;;
        status.url) _m='地址       %s' ;;
        status.method) _m='运行方式   %s' ;;
        status.listen) _m='监听       %s' ;;
        status.version) _m='版本       %s' ;;
        status.health) _m='健康检查   %s' ;;
        status.quic) _m='QUIC 证书  %s' ;;
        status.control) _m='控制面     %s' ;;
        status.ok) _m='正常响应' ;;
        status.down) _m='没有响应' ;;
        status.present) _m='已就位' ;;
        status.missing) _m='缺失：设备只能经 relay 中转' ;;
        status.behind) _m='%s，在代理之后' ;;
        menu.q) _m='要做什么？' ;;
        menu.upgrade) _m='升级' ;;
        menu.upgrade.hint) _m='从控制面更新 relay 与本脚本' ;;
        menu.listen) _m='更改监听地址' ;;
        menu.listen.hint) _m='代理转发到的地址' ;;
        menu.uninstall) _m='卸载' ;;
        menu.uninstall.hint) _m='从这台机器删除节点' ;;
        menu.quit) _m='退出' ;;
        upgrade.http) _m='控制面是明文 http://，中间人可以给这台机器一个以 root 运行的脚本；请经 https:// 升级，或加 --allow-http' ;;
        upgrade.fetch) _m='从控制面获取安装脚本' ;;
        upgrade.notours) _m='控制面返回的不是 yttt 安装脚本' ;;
        upgrade.older) _m='控制面提供的是 %s，比本节点（%s）旧；请先升级控制面，或加 --force' ;;
        upgrade.handover) _m='交给 %s 的安装脚本继续' ;;
        upgrade.done) _m='已升级到 %s' ;;
        listen.colocated) _m='这个节点在控制面的 Caddy 之后，Caddy 固定转发到 127.0.0.1:3340，地址不能改' ;;
        listen.direct) _m='这个节点直接对外提供服务；监听地址只用于代理之后的节点' ;;
        listen.done) _m='节点改为监听 %s；请让代理转发到这里。' ;;
        uninstall.q) _m='从这台机器删除节点？将删除：' ;;
        uninstall.what) _m='%s、%s 中的配置与 %s 中的状态' ;;
        uninstall.done) _m='已删除；请在控制台停用或删除这个节点' ;;
        cancelled) _m='没有做任何改动。' ;;
        *) _m='' ;;
    esac
    printf '%s' "$_m"
}

# --- Questions --------------------------------------------------------------------

ask_method() {
    ui_select method "$(t method.q)" native \
        "native|$(t method.native)|$(t method.native.hint)" \
        "docker|$(t method.docker)|$(t method.docker.hint)"
}

ask_domain() {
    [ -z "$domain" ] || return 0
    ui_interactive || return 0
    ui_note "$(t domain.about)" >/dev/tty
    ui_input domain "$(t domain.q)" ''
}

# A node with a domain serves HTTPS on 443 and answers certificate
# challenges on 80. When another program holds them, the node goes behind
# it or does without the domain; its own port must be free either way.
check_ports() {
    UI_OURS=yttt-relay
    if [ -n "$domain" ] && [ "$behind_proxy" = 0 ]; then
        owner=$(ui_port_owner 0.0.0.0 443)
        [ -n "$owner" ] || owner=$(ui_port_owner 0.0.0.0 80)
        case "$owner" in
            '') ;;
            '!') ui_warn "$(t ports.unknown)" ;;
            *)
                owner=$(ui_owner_name "$owner")
                ui_interactive || die "$(t ports.taken.yes "$owner")"
                ui_warn "$(t ports.taken "$owner" "$domain")"
                serve=''
                ui_select serve "$(t ports.q "$domain")" behind \
                    "behind|$(t ports.behind "$owner")|$(t ports.behind.hint)" \
                    "nodomain|$(t ports.nodomain)|$(t ports.nodomain.hint)"
                if [ "$serve" = behind ]; then behind_proxy=1; else domain=''; fi
                ;;
        esac
    fi
    if [ "$behind_proxy" = 1 ]; then
        ui_listen listen 127.0.0.1 3340
    elif [ -z "$domain" ]; then
        _ui_listen_addr=0.0.0.0
        ui_input port "$(t port.q)" 3341 _ui_check_port
    fi
}

# --- Main -------------------------------------------------------------------------

main() {
    domain=''
    public_url=''
    binary_url=''
    image=''
    method=''
    no_systemd=0
    behind_proxy=0
    listen=''
    port=''
    yes=0
    force=0
    allow_http=0
    mode=''
    while [ $# -gt 0 ]; do
        case "$1" in
            --domain | --public-url | --binary-url | --image | --method | --listen | --port | --lang)
                [ $# -ge 2 ] || die "$(t bad.value "$1")"
                case "$1" in
                    --domain) domain=$2 ;;
                    --public-url) public_url=$2 ;;
                    --binary-url) binary_url=$2 ;;
                    --image) image=$2 ;;
                    --method) method=$2 ;;
                    --listen) listen=$2 ;;
                    --port) port=$2 ;;
                    --lang) YTTT_LANG=$2 ;;
                esac
                shift 2
                ;;
            --no-systemd) no_systemd=1; shift ;;
            --behind-proxy) behind_proxy=1; shift ;;
            --yes) yes=1; shift ;;
            --force) force=1; shift ;;
            --allow-http) allow_http=1; shift ;;
            --upgrade) mode=upgrade; shift ;;
            --uninstall) mode=uninstall; shift ;;
            *) die "$(t bad.option "$1")" ;;
        esac
    done
    # A handed-over script removes its downloaded copy when it is done.
    [ -z "${YTTT_HANDOFF_FILE:-}" ] || ui_on_exit 'rm -f "$YTTT_HANDOFF_FILE"'

    ui_init "$yes"
    ui_choose_language
    [ "$(id -u)" = 0 ] || die "$(t need.root)"
    [ "$(uname -s)" = Linux ] || die "$(t need.linux)"
    case "$(uname -m)" in
        x86_64 | amd64) arch=x86_64 ;;
        aarch64 | arm64) arch=aarch64 ;;
        *) die "$(t need.arch "$(uname -m)")" ;;
    esac
    command -v curl >/dev/null || die "$(t need.curl)"

    # An existing node remembers how it was installed.
    installed=$(awk -F= '$1 == "METHOD" { print $2 }' "$CONFIG_DIR/install.env" 2>/dev/null || true)
    enrolled=0
    [ ! -f "$CONFIG_DIR/relay.toml" ] || enrolled=1
    if [ -z "$mode" ]; then
        if [ -n "${YTTT_ENROLL:-}" ]; then
            mode=install
        elif [ "$enrolled" = 1 ] && [ -n "$listen" ]; then
            mode=listen
        elif [ "$enrolled" = 1 ]; then
            mode=manage
        else
            die "$(t need.enroll)"
        fi
    fi
    [ "$mode" = install ] || [ "$enrolled" = 1 ] || die "$(t need.node)"
    [ "${YTTT_HANDOFF:-}" = 1 ] || ui_banner "$(t title)" "$VERSION · $CONTROL"

    case "$mode" in
        install) install_node ;;
        upgrade) upgrade ;;
        uninstall) uninstall_node ;;
        listen) change_listen ;;
        manage) manage ;;
    esac
}

install_node() {
    ask_method
    case "$method" in native | docker) ;; *) die "$(t bad.method)" ;; esac
    ask_domain
    [ "$behind_proxy" = 0 ] || [ -n "$domain" ] || die "$(t bad.proxy)"
    [ -z "$listen" ] || [ "$behind_proxy" = 1 ] || die "$(t bad.listen)"
    [ -z "$port" ] || [ -z "$domain" ] || die "$(t bad.port)"
    check_ports
    if [ -n "$domain" ] && command -v getent >/dev/null && ! getent hosts "$domain" >/dev/null; then
        die "$(t bad.resolve "$domain")"
    fi
    ui_detach
    mkdir -p "$CONFIG_DIR"
    chmod 755 "$CONFIG_DIR"
    if [ "$method" = docker ]; then
        docker_install
    else
        native_install
    fi
    {
        echo "METHOD=$method"
        echo "PROXIED=$behind_proxy"
    } >"$CONFIG_DIR/install.env"
    keep_installer
    wait_healthy
    summary
}

enroll_args() {
    set -- --control "$CONTROL" --config-dir "$CONFIG_DIR"
    [ -z "$domain" ] || set -- "$@" --domain "$domain"
    [ -z "$public_url" ] || set -- "$@" --public-url "$public_url"
    [ "$behind_proxy" = 0 ] || set -- "$@" --behind-proxy \
        --quic-cert "$CONFIG_DIR/quic/cert.pem" --quic-key "$CONFIG_DIR/quic/key.pem"
    [ -z "$listen" ] || set -- "$@" --listen "$listen"
    [ -z "$port" ] || set -- "$@" --port "$port"
    printf '%s\n' "$@"
}

# The token goes over stdin, never onto a command line.
native_enroll() {
    # shellcheck disable=SC2046
    printf '%s' "$YTTT_ENROLL" | "$PREFIX/yttt-relay" enroll $(enroll_args)
}

native_install() {
    command -v sha256sum >/dev/null || die "$(t need.sha)"
    ui_spin "$(t step.download)" fetch_binary "$arch" "$binary_url" || exit 1
    ui_spin "$(t step.enroll "$CONTROL")" native_enroll || exit 1
    unset YTTT_ENROLL

    if [ "$no_systemd" = 1 ]; then
        keep_installer
        ui_step "$(t foreground)"
        ui_before_exec
        exec "$PREFIX/yttt-relay" --config "$CONFIG_DIR/relay.toml"
    fi
    command -v systemctl >/dev/null || die "$(t need.systemd)"
    # The service runs as its own user, which may read the token but not change it.
    if ! id yttt-relay >/dev/null 2>&1; then
        useradd --system --no-create-home --home-dir /nonexistent --shell /usr/sbin/nologin yttt-relay
    fi
    chown root:yttt-relay "$CONFIG_DIR/node-token"
    chmod 640 "$CONFIG_DIR/node-token"
    install_unit
    systemctl daemon-reload
    systemctl enable yttt-relay >/dev/null 2>&1
    # A machine deployed again keeps its service running; restart it so the
    # new enrollment takes effect.
    systemctl restart yttt-relay
}

# Host networking, like the node: the control plane may be on this machine.
docker_enroll() {
    # shellcheck disable=SC2046
    printf '%s' "$YTTT_ENROLL" | docker run --rm -i --user 0:0 --network host \
        -v "$CONFIG_DIR:$CONFIG_DIR" --entrypoint yttt-relay "$image" enroll $(enroll_args)
}

docker_install() {
    command -v docker >/dev/null || die "$(t need.docker)"
    image=${image:-$IMAGE_DEFAULT}
    ui_spin "$(t step.pull "$image")" pull "$image" || exit 1
    mkdir -p "$STATE_DIR"
    ui_spin "$(t step.enroll "$CONTROL")" docker_enroll || exit 1
    unset YTTT_ENROLL
    chmod 600 "$CONFIG_DIR/node-token"
    echo "IMAGE=$image" >"$CONFIG_DIR/docker.env"
    ui_spin "$(t step.start)" run_container "$image" || exit 1
}

# Fetches the image, or uses a copy already on this machine.
pull() {
    docker pull -q "$1" >/dev/null 2>&1 \
        || docker image inspect "$1" >/dev/null 2>&1 \
        || die "$(t bad.pull "$1")"
}

# Starts the node's container, replacing an earlier one.
run_container() {
    docker rm -f "$CONTAINER" >/dev/null 2>&1 || true
    # Host networking: QUIC address discovery needs the machine's own
    # addresses, and the node binds 80/443 when it has a domain. Docker gives
    # added capabilities only to root, so it runs as root with nothing else.
    docker run -d --name "$CONTAINER" --restart unless-stopped --network host \
        --user 0:0 --cap-drop ALL --cap-add NET_BIND_SERVICE \
        -v "$CONFIG_DIR:$CONFIG_DIR:ro" -v "$STATE_DIR:$STATE_DIR" \
        --entrypoint yttt-relay "$1" --config "$CONFIG_DIR/relay.toml" >/dev/null
}

# Keeps this script as the node's management command. Run through a pipe
# there is no file to copy, so it fetches the control plane's.
keep_installer() {
    if [ -f "$0" ] && grep -q '^# yttt-installer-protocol:' "$0" 2>/dev/null; then
        source=$0
    else
        source=$(mktemp)
        ui_on_exit "rm -f '$source'"
        curl -fsSL "$CONTROL/install/relay.sh" -o "$source" || die "$(t bad.download "$CONTROL/install/relay.sh")"
    fi
    cmp -s "$source" "$MANAGER" 2>/dev/null || install -m 755 "$source" "$MANAGER"
}

fetch_binary() {
    temporary=$(mktemp -d)
    if [ -n "$2" ]; then
        curl -fsSL "$2" -o "$temporary/yttt-relay" || { rm -rf "$temporary"; die "$(t bad.download "$2")"; }
    else
        name="yttt-relay-linux-$1"
        if ! curl -fsSL "$CONTROL/install/bin/$name" -o "$temporary/yttt-relay" \
            || ! curl -fsSL "$CONTROL/install/bin/SHA256SUMS" -o "$temporary/SHA256SUMS"; then
            rm -rf "$temporary"
            die "$(t bad.download "$CONTROL/install/bin/$name")"
        fi
        expected=$(awk -v name="$name" '$2 == name { print $1 }' "$temporary/SHA256SUMS")
        [ -n "$expected" ] || { rm -rf "$temporary"; die "$(t bad.checksum.list "$name")"; }
        actual=$(sha256sum "$temporary/yttt-relay" | awk '{ print $1 }')
        [ "$expected" = "$actual" ] || { rm -rf "$temporary"; die "$(t bad.checksum)"; }
    fi
    install -m 755 "$temporary/yttt-relay" "$PREFIX/yttt-relay"
    rm -rf "$temporary"
}

install_unit() {
    cat >"$UNIT" <<'UNIT_EOF'
[Unit]
Description=yttt relay node
After=network-online.target
Wants=network-online.target

[Service]
ExecStart=/usr/local/bin/yttt-relay --config /etc/yttt-relay/relay.toml
User=yttt-relay
Group=yttt-relay
# Certificates from Let's Encrypt persist here across restarts.
StateDirectory=yttt-relay
AmbientCapabilities=CAP_NET_BIND_SERVICE
CapabilityBoundingSet=CAP_NET_BIND_SERVICE
PrivateTmp=yes
NoNewPrivileges=yes
ProtectSystem=strict
ProtectHome=yes
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
UNIT_EOF
}

# --- The installed node -------------------------------------------------------------

config_value() {
    awk -F'"' -v key="$1" '$1 ~ "^" key "[ ]*=" { print $2; exit }' "$CONFIG_DIR/relay.toml"
}

proxied() {
    grep -q '^\[quic_tls\]' "$CONFIG_DIR/relay.toml"
}

# Where this machine reaches the relay itself.
local_address() {
    address=$(config_value listen)
    host=${address%:*}
    case "$host" in 0.0.0.0 | '[::]' | '') host=127.0.0.1 ;; esac
    echo "$host:${address##*:}"
}

# Another program may answer on the port; the relay says "ok".
answering() {
    target=$(local_address)
    [ "$(curl -fsS -k -m 3 "http://$target/healthz" 2>/dev/null)" = ok ] \
        || [ "$(curl -fsS -k -m 3 "https://$target/healthz" 2>/dev/null)" = ok ]
}

wait_until_answering() {
    i=0
    while [ $i -lt 30 ]; do
        answering && return 0
        i=$((i + 1))
        sleep 1
    done
    return 1
}

logs_hint() {
    if [ "$installed" = docker ] || [ "$method" = docker ]; then
        echo "docker logs $CONTAINER"
    else
        echo "journalctl -u yttt-relay"
    fi
}

wait_healthy() {
    if ! ui_spin "$(t step.wait)" wait_until_answering; then
        ui_warn "$(t warn.noanswer "$(logs_hint)")"
    fi
}

summary() {
    url=$(awk -F'"' '/^# url/ { print $2 }' "$CONFIG_DIR/relay.toml")
    address=$(config_value listen)
    set -- "$(t done.online)"
    if [ "$behind_proxy" = 1 ]; then
        set -- "$(t done.proxy "$domain" "$address")" "$(t done.quic "$domain" "$CONFIG_DIR")" "$@"
    elif [ -n "$domain" ]; then
        set -- "$(t done.domain)" "$@"
    else
        set -- "$(t done.plain "${address##*:}")" "$@"
    fi
    ui_box "$(t done.title "${url:-$(config_value node_id)}")" "$@" "$(t done.manage "$MANAGER")"
}

status() {
    node=$(config_value node_id)
    url=$(awk -F'"' '/^# url/ { print $2 }' "$CONFIG_DIR/relay.toml")
    address=$(config_value listen)
    if [ "$installed" = docker ]; then
        how=$(t method.docker)
        version=$(awk -F= '$1 == "IMAGE" { print $2 }' "$CONFIG_DIR/docker.env" 2>/dev/null || true)
    else
        how=$(t method.native)
        version=$("$PREFIX/yttt-relay" --version 2>/dev/null || true)
    fi
    if proxied; then how=$(t status.behind "$how"); fi
    if answering; then health=$(t status.ok); else health="$(t status.down) ($(logs_hint))"; fi
    set -- "$(t status.url "$url")" "$(t status.method "$how")" "$(t status.listen "$address")" \
        "$(t status.version "${version:-?}")" "$(t status.health "$health")"
    if proxied; then
        if [ -f "$CONFIG_DIR/quic/cert.pem" ]; then cert=$(t status.present); else cert=$(t status.missing); fi
        set -- "$@" "$(t status.quic "$cert")"
    fi
    ui_box "$(t status.title "$node")" "$@" "$(t status.control "$CONTROL")"
}

manage() {
    status
    ui_interactive || return 0
    action=''
    if proxied && [ ! -f "$COLOCATED_SYNC" ]; then
        ui_select action "$(t menu.q)" upgrade \
            "upgrade|$(t menu.upgrade)|$(t menu.upgrade.hint)" \
            "listen|$(t menu.listen)|$(t menu.listen.hint)" \
            "uninstall|$(t menu.uninstall)|$(t menu.uninstall.hint)" \
            "quit|$(t menu.quit)"
    else
        ui_select action "$(t menu.q)" upgrade \
            "upgrade|$(t menu.upgrade)|$(t menu.upgrade.hint)" \
            "uninstall|$(t menu.uninstall)|$(t menu.uninstall.hint)" \
            "quit|$(t menu.quit)"
    fi
    case "$action" in
        upgrade) upgrade ;;
        listen) change_listen ;;
        uninstall) uninstall_node ;;
    esac
}

# Upgrades with the control plane's current script: this one hands over
# unless it is that script already.
upgrade() {
    if [ "${YTTT_HANDOFF:-}" != 1 ]; then
        case "$CONTROL" in
            https://*) ;;
            *) [ "$allow_http" = 1 ] || die "$(t upgrade.http)" ;;
        esac
        fresh=$(mktemp)
        ui_on_exit "rm -f '$fresh'"
        ui_spin "$(t upgrade.fetch)" curl -fsSL "$CONTROL/install/relay.sh" -o "$fresh" || exit 1
        grep -q '^#!/bin/sh' "$fresh" || die "$(t upgrade.notours)"
        offered=$(sed -n "s/^VERSION='\\(.*\\)'\$/\\1/p" "$fresh" | head -n 1)
        if [ "$force" = 0 ]; then
            # A script without the protocol marker predates this one.
            if ! grep -q '^# yttt-installer-protocol:' "$fresh"; then
                die "$(t upgrade.older "${offered:-0.5.0}" "$VERSION")"
            fi
            case "$VERSION$offered" in
                *__*) ;;
                *) ! ui_version_lt "$offered" "$VERSION" || die "$(t upgrade.older "$offered" "$VERSION")" ;;
            esac
        fi
        if ! cmp -s "$fresh" "$0"; then
            ui_step "$(t upgrade.handover "${offered:-?}")"
            set -- --upgrade
            [ "$yes" = 0 ] || set -- "$@" --yes
            [ "$force" = 0 ] || set -- "$@" --force
            [ -z "$image" ] || set -- "$@" --image "$image"
            [ -z "$binary_url" ] || set -- "$@" --binary-url "$binary_url"
            handoff=$(mktemp)
            cp "$fresh" "$handoff"
            ui_before_exec
            YTTT_HANDOFF=1 YTTT_HANDOFF_FILE=$handoff exec sh "$handoff" "$@"
        fi
    fi
    ui_detach
    if [ "$installed" = docker ]; then
        image=${image:-$IMAGE_DEFAULT}
        ui_spin "$(t step.pull "$image")" pull "$image" || exit 1
        echo "IMAGE=$image" >"$CONFIG_DIR/docker.env"
        ui_spin "$(t step.start)" run_container "$image" || exit 1
        now=$image
    else
        command -v sha256sum >/dev/null || die "$(t need.sha)"
        ui_spin "$(t step.download)" fetch_binary "$arch" "$binary_url" || exit 1
        [ ! -f "$UNIT" ] || systemctl restart yttt-relay
        now=$("$PREFIX/yttt-relay" --version 2>/dev/null || echo "$VERSION")
    fi
    keep_installer
    wait_healthy
    ui_ok "$(t upgrade.done "$now")"
}

change_listen() {
    [ ! -f "$COLOCATED_SYNC" ] || die "$(t listen.colocated)"
    proxied || die "$(t listen.direct)"
    current=$(config_value listen)
    UI_OURS=yttt-relay
    ui_listen listen "${current%:*}" "${current##*:}"
    sed -i "s|^listen = \".*\"|listen = \"$listen\"|" "$CONFIG_DIR/relay.toml"
    if [ "$installed" = docker ]; then
        docker restart "$CONTAINER" >/dev/null
    else
        systemctl restart yttt-relay
    fi
    wait_healthy
    ui_ok "$(t listen.done "$listen")"
}

uninstall_node() {
    if ui_interactive; then
        ui_warn "$(t uninstall.q)"
        ui_note "$(t uninstall.what "$PREFIX/yttt-relay" "$CONFIG_DIR" "$STATE_DIR")"
        if ! ui_confirm "$(t menu.uninstall)?" no; then
            ui_note "$(t cancelled)"
            return 0
        fi
    fi
    ui_detach
    if [ "$installed" = docker ]; then
        docker rm -f "$CONTAINER" >/dev/null 2>&1 || true
    elif [ -f "$UNIT" ]; then
        systemctl disable --now yttt-relay >/dev/null 2>&1 || true
        rm -f "$UNIT"
        systemctl daemon-reload || true
    fi
    rm -f "$PREFIX/yttt-relay" "$MANAGER"
    rm -rf "$CONFIG_DIR" "$STATE_DIR"
    if [ "$installed" != docker ] && id yttt-relay >/dev/null 2>&1; then
        userdel yttt-relay || true
    fi
    ui_ok "$(t uninstall.done)"
}

main "$@"
