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
