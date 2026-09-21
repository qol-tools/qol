#!/bin/sh
set -u
: "${WAVE2_WORKFLOW_ID:?scenario requires WAVE2_WORKFLOW_ID}"

TRAY=/usr/lib/qol-tray/qol-resident-policy
POLICY=nvidia-driver-version-pin
FRAGMENT=/etc/apt/preferences.d/90qol-nvidia-driver.pref
JOURNAL=/var/lib/qol-resident-policy-nvidia-driver-version-pin.json
JOURNAL_STAGE=/var/lib/.qol-resident-policy-nvidia-driver-version-pin.json.stage
CONTROL=fixture-ctl
OWNER_B=test-owner-b
PAYLOAD_ROOT=/qol-payload
PHASE=${1:-phase1}
REAL_CORE_DEPS="libc6 libgcc-s1 libstdc++6 libselinux1 libpcre2-8-0 libzstd1 liblzma5 libbz2-1.0 libcap2 libmount1 libblkid1 libuuid1 libacl1 libattr1 liblz4-1 libffi8 libgcrypt20 libgpg-error0 libsystemd0 libdbus-1-3 libexpat1 libmd0 libbsd0 libxau6 libxdmcp6 zlib1g"
FIXTURE_DEPS="libgtk-3-0 libgtk-3-0t64 libayatana-appindicator3-1 libglib2.0-0 libglib2.0-0t64 libgdk-pixbuf-2.0-0 libgdk-pixbuf-2.0-0t64 libcairo2 libcairo-gobject2 libpango-1.0-0 libpango-1.0-0t64 libpangocairo-1.0-0 libpangocairo-1.0-0t64 libpangoft2-1.0-0 libpangoft2-1.0-0t64 libharfbuzz0b libfontconfig1 libfreetype6 libfribidi0 libpixman-1-0 libpng16-16 libjpeg62-turbo libx11-6 libxext6 libxdamage1 libxcomposite1 libxfixes3 libxrender1 libxinerama1 libxi6 libxtst6 libxcursor1 libxrandr2 libxcb-render0 libxcb-shm0 libxcb-xkb1 libxcb1 libxkbcommon-x11-0 libxkbcommon0 libepoxy0 libatk1.0-0 libatk1.0-0t64 libatk-bridge2.0-0 libatk-bridge2.0-0t64 libatspi2.0-0 libwayland-client0 libwayland-cursor0 libwayland-egl1 libxdo3 libbrotli1 libgraphite2-3 libdatrie1 libthai0 libgmodule-2.0-0 libgobject-2.0-0"
PROVIDER=qol-headless-deps
bootstrap_fail() {
    echo "w2 bootstrap failure: $*" >&2
    exit 1
}

check_workflow_id() {
    manifest=$1
    [ -f "$manifest" ] && [ ! -L "$manifest" ] ||
        bootstrap_fail "payload manifest must be a regular file"
    n=$(grep -o '"workflow_id"[[:space:]]*:[[:space:]]*"[^"]*"' "$manifest" 2>/dev/null | wc -l)
    [ $((n)) -eq 1 ] || bootstrap_fail "payload manifest workflow_id count=$n"
    v=$(grep -o '"workflow_id"[[:space:]]*:[[:space:]]*"[^"]*"' "$manifest" |
        sed -n 's/^"workflow_id"[[:space:]]*:[[:space:]]*"\(.*\)"$/\1/p' | head -1)
    [ "$v" = "$WAVE2_WORKFLOW_ID" ] ||
        bootstrap_fail "payload workflow mismatch: $v"
}

mount_payload() {
    mkdir -p "$PAYLOAD_ROOT"
    if ! grep -q " $PAYLOAD_ROOT " /proc/mounts; then
        mount -t iso9660 -o ro LABEL=QOL_PAYLOAD "$PAYLOAD_ROOT" ||
            bootstrap_fail "payload mount"
    fi
    check_workflow_id "$PAYLOAD_ROOT/manifest.json"
}

install_modinfo_fixture() {
    if [ -f "$STAGE/modinfo.fixture" ]; then
        [ -f "$STAGE/modinfo.fixture-script" ] ||
            fail_hard "the modinfo fixture evidence is incomplete; refusing to reinstall over the live entry"
        if [ -L /usr/sbin/modinfo ]; then
            fail_hard "the live /usr/sbin/modinfo is a symlink while the fixture marker exists; refusing to overwrite it"
        fi
        if [ ! -f /usr/sbin/modinfo ]; then
            fail_hard "the live /usr/sbin/modinfo is absent or not a regular file while the fixture marker exists; refusing to recreate it"
        fi
        if ! cmp -s /usr/sbin/modinfo "$STAGE/modinfo.fixture-script"; then
            fail_hard "the live /usr/sbin/modinfo no longer matches the recorded fixture; refusing to overwrite the operator replacement"
        fi
        return
    fi
    if [ -e /usr/sbin/modinfo ] || [ -L /usr/sbin/modinfo ]; then
        mv /usr/sbin/modinfo "$STAGE/modinfo.original"
    fi
    cat > "$STAGE/modinfo.fixture-script" <<'FIXEOF'
#!/bin/sh
case "$1" in
    -n)
        exit 1
        ;;
    -F)
        if [ "$2" = version ]; then
            echo "580.159.02"
            exit 0
        fi
        exit 1
        ;;
    *)
        exit 1
        ;;
esac
FIXEOF
    chmod 755 "$STAGE/modinfo.fixture-script"
    cp "$STAGE/modinfo.fixture-script" /usr/sbin/modinfo
    chmod 755 /usr/sbin/modinfo
    touch "$STAGE/modinfo.fixture"
}

restore_modinfo_fixture() {
    [ -f "$STAGE/modinfo.fixture" ] || return 0
    ours=no
    if [ -f /usr/sbin/modinfo ] && cmp -s /usr/sbin/modinfo "$STAGE/modinfo.fixture-script"; then
        ours=yes
    fi
    if [ "$ours" = yes ]; then
        rm -f /usr/sbin/modinfo
        if [ -f "$STAGE/modinfo.original" ]; then
            mv "$STAGE/modinfo.original" /usr/sbin/modinfo
        fi
        rm -f "$STAGE/modinfo.fixture" "$STAGE/modinfo.fixture-script"
        return 0
    fi
    return 1
}

headless_dep_name() {
    printf '%s' "$1" | sed 's/|.*//;s/(.*//' | tr -d ' '
}

headless_dep_constraint() {
    case "$1" in
        *"("*")"*) echo yes ;;
        *) echo no ;;
    esac
}

adapter_unresolved_libs() {
    [ -x /usr/lib/qol-tray/qol-resident-policy ] ||
        fail_hard "the installed payload is missing the resident adapter"
    missing=$(ldd /usr/lib/qol-tray/qol-resident-policy 2>/dev/null | grep -c 'not found' || true)
    echo "$missing"
}

install_headless_deps_provider() {
    deps_file=$STAGE/deb-deps.txt
    dpkg-deb -f "$PAYLOAD_ROOT/qol-tray.deb" Depends |
        tr ',' '\n' | sed 's/^ *//;s/ *$//' > "$deps_file"
    provides_list=""
    while IFS= read -r part; do
        [ -n "$part" ] || continue
        name=$(headless_dep_name "$part")
        case "$name" in
            *[!a-z0-9+.-]*)
                fail_hard "malformed dependency name in: $part"
                ;;
        esac
        if printf '%s' "$REAL_CORE_DEPS" | tr ' ' '\n' | grep -Fxq "$name"; then
            continue
        fi
        if printf '%s' "$FIXTURE_DEPS" | tr ' ' '\n' | grep -Fxq "$name"; then
            if [ "$(headless_dep_constraint "$part")" = yes ]; then
                version=$(printf '%s' "$part" | sed 's/.*(//;s/).*//' | awk '{print $NF}')
                [ -n "$version" ] || fail_hard "malformed dependency constraint in: $part"
                provides_list="$provides_list$name (= $version), "
            else
                provides_list="$provides_list$name, "
            fi
            continue
        fi
        fail_hard "dependency drift: the production deb depends on unapproved package '$name'"
    done < "$deps_file"
    d=$STAGE/build/$PROVIDER
    mkdir -p "$d/DEBIAN" "$d/usr/share/$PROVIDER"
    {
        echo "Package: $PROVIDER"
        echo "Version: 1.0"
        echo "Section: misc"
        echo "Priority: optional"
        echo "Architecture: all"
        echo "Maintainer: wave2 gate <wave2@qol.invalid>"
        echo "Description: guest-only headless provider for nonessential GUI dependencies of the production qol-tray deb; never ships and never provides core libraries"
        if [ -n "$provides_list" ]; then
            echo "Provides: ${provides_list%, }"
        fi
    } > "$d/DEBIAN/control"
    dpkg-deb --build "$d" "$STAGE/${PROVIDER}_1.0_all.deb" >/dev/null 2>>"$LOG" ||
        fail_hard "dpkg-deb failed for the headless dependency provider"
    dpkg -i "$STAGE/${PROVIDER}_1.0_all.deb" >/dev/null 2>>"$LOG" ||
        fail_hard "headless dependency provider install"
}

require_apt_health() {
    apt_check_log=$STAGE/apt-check.log
    apt-get check >"$apt_check_log" 2>&1
    rc=$?
    if [ $rc -ne 0 ]; then
        if command -v tail >/dev/null 2>&1 && [ -r "$apt_check_log" ]; then
            {
                echo "w2 apt-check tail:"
                tail -n 5 "$apt_check_log"
            } >>"$LOG" 2>/dev/null || true
        fi
        fail_hard "apt-get check failed after the production deb install"
    fi
    rm -f "$apt_check_log" || fail_hard "apt-check evidence cleanup failed after the production deb install"
    audit_lines=$(dpkg --audit 2>/dev/null | grep -c . || true)
    [ "$audit_lines" = 0 ] ||
        fail_hard "dpkg --audit reported $audit_lines inconsistent packages after the production deb install"
}

stage_path_owned() {
    case "$1" in
        /var/tmp/w2.[A-Za-z0-9][A-Za-z0-9][A-Za-z0-9][A-Za-z0-9][A-Za-z0-9][A-Za-z0-9] | \
        /var/tmp/w2c.[A-Za-z0-9][A-Za-z0-9][A-Za-z0-9][A-Za-z0-9][A-Za-z0-9][A-Za-z0-9]) return 0 ;;
    esac
    return 1
}

remove_owned_stage() {
    path=$1
    if stage_path_owned "$path" && [ -d "$path" ] && [ ! -L "$path" ] \
        && rm -rf "$path" && [ ! -e "$path" ] && [ ! -L "$path" ]; then
        return 0
    fi
    return 1
}

case "$PHASE" in
    phase1)
        mount_payload
        STAGE=$(mktemp -d /var/tmp/w2.XXXXXX) || exit 1
        stage_path_owned "$STAGE" || exit 1
        [ -d "$STAGE" ] && [ ! -L "$STAGE" ] || exit 1
        printf '%s' "$STAGE" > /mnt/w2-stage-path
        ;;
    contract)
        mount_payload
        STAGE=$(mktemp -d /var/tmp/w2c.XXXXXX) || exit 1
        stage_path_owned "$STAGE" || exit 1
        [ -d "$STAGE" ] && [ ! -L "$STAGE" ] || exit 1
        [ -f /etc/apt/sources.list ] && mv /etc/apt/sources.list "$STAGE/sources.list.off"
        for f in /etc/apt/sources.list.d/*; do
            [ -e "$f" ] || continue
            mv "$f" "$STAGE/"
        done
        ;;
    phase2)
        mount_payload
        STAGE=$(cat /mnt/w2-stage-path 2>/dev/null || true)
        stage_path_owned "$STAGE" || exit 1
        [ -d "$STAGE" ] && [ ! -L "$STAGE" ] || exit 1
        ;;
esac
REPO=$STAGE/repo
RESULTS=/mnt/results.json
[ "$PHASE" = phase1 ] && RESULTS=/mnt/results-phase1.json
LOG=$STAGE/scenario.log
install_modinfo_fixture

log() { echo "w2: $*" >> "$LOG"; }

record() {
    id=$1
    pass=$2
    detail=$3
    detail=$(printf '%s' "$detail" | tr -d '\000-\037\177' | cut -c1-200 | sed 's/\\/\\\\/g;s/"/\\"/g')
    if [ "$pass" = 1 ]; then
        verdict=true
    else
        verdict=false
    fi
    printf '{"id":"%s","pass":%s,"detail":"%s"}\n' "$id" "$verdict" "$detail" >> "$RESULTS"
}

fail_hard() {
    log "internal failure: $*"
    record "internal" 0 "$*"
    echo "w2 internal failure: $*" >&2
    if command -v tail >/dev/null 2>&1 && [ -f "$LOG" ] && [ -r "$LOG" ]; then
        {
            echo "w2 scenario log tail:"
            tail -n 40 "$LOG"
        } >&2 2>/dev/null || true
    fi
    exit 1
}

build_deb() {
    name=$1
    ver=$2
    d=$STAGE/build/$name-$ver
    mkdir -p "$d/DEBIAN" "$d/usr/share/$name"
    {
        echo "Package: $name"
        echo "Version: $ver"
        echo "Section: misc"
        echo "Priority: optional"
        echo "Architecture: all"
        echo "Maintainer: wave2 gate <wave2@qol.invalid>"
        echo "Description: wave2 gate fixture"
    } > "$d/DEBIAN/control"
    echo "$ver" > "$d/usr/share/$name/version"
    dpkg-deb --build "$d" "$REPO/pool/${name}_${ver}_all.deb" >/dev/null 2>>"$LOG" ||
        fail_hard "dpkg-deb failed for $name $ver"
}

gen_packages() {
    cd "$REPO" || fail_hard "repo dir missing"
    : > Packages
    for f in pool/*.deb; do
        [ -f "$f" ] || continue
        name=$(dpkg-deb -f "$f" Package) || fail_hard "dpkg-deb -f Package $f"
        ver=$(dpkg-deb -f "$f" Version) || fail_hard "dpkg-deb -f Version $f"
        size=$(stat -c %s "$f") || fail_hard "stat $f"
        sha=$(sha256sum "$f" | cut -d' ' -f1) || fail_hard "sha256sum $f"
        {
            echo "Package: $name"
            echo "Version: $ver"
            echo "Architecture: all"
            echo "Filename: $f"
            echo "Size: $size"
            echo "SHA256: $sha"
            echo "Description: wave2 gate fixture"
            echo
        } >> Packages
    done
    gzip -kf Packages
}

gen_release() {
    cd "$REPO" || return 44
    now=$(date -u +%a,\ %d\ %b\ %Y\ %H:%M:%S\ UTC)
    sha_packages=$(sha256sum Packages | cut -d' ' -f1) || return 44
    size_packages=$(stat -c %s Packages) || return 44
    sha_packages_gz=$(sha256sum Packages.gz | cut -d' ' -f1) || return 44
    size_packages_gz=$(stat -c %s Packages.gz) || return 44
    {
        echo "Origin: wave2"
        echo "Label: wave2"
        echo "Suite: now"
        echo "Codename: now"
        echo "Architectures: all"
        echo "Components: ."
        echo "Date: $now"
        echo "SHA256:"
        echo " $sha_packages $size_packages Packages"
        echo " $sha_packages_gz $size_packages_gz Packages.gz"
    } > Release
    return 0
}

setup() {
    mkdir -p "$REPO/pool" "$STAGE/aptbak"
    for pkg in nvidia-driver-fixture-a nvidia-driver-fixture-b fixture-ctl; do
        build_deb "$pkg" 1.0
    done
    build_deb "$CONTROL" 2.0
    build_deb "$CONTROL" 3.0
    build_deb "$CONTROL" 4.0
    build_deb nvidia-driver-fixture-a 2.0
    build_deb nvidia-driver-fixture-b 2.0
    gen_packages
    gen_release

    mkdir -p /etc/apt/sources.list.d
    systemctl stop apt-daily.timer apt-daily-upgrade.timer unattended-upgrades.timer \
        unattended-upgrades.service apt-daily.service apt-daily-upgrade.service 2>/dev/null || true
    systemctl mask apt-daily.timer apt-daily-upgrade.timer unattended-upgrades.timer \
        2>/dev/null || true
    pkill -f unattended-upgrade 2>/dev/null || true
    mv /etc/apt/sources.list "$STAGE/aptbak/" 2>/dev/null || true
    for f in /etc/apt/sources.list.d/*; do
        [ -f "$f" ] && mv "$f" "$STAGE/aptbak/" 2>/dev/null || true
    done
    echo "deb [trusted=yes] file://$REPO ./" > /etc/apt/sources.list.d/wave2-local.list
    apt-get update -qq >/dev/null 2>>"$LOG" || fail_hard "apt-get update"
    apt-get install -y -qq nvidia-driver-fixture-a=1.0 nvidia-driver-fixture-b=1.0 fixture-ctl=1.0 \
        >/dev/null 2>>"$LOG" || fail_hard "fixture install"
}

version_of() {
    dpkg-query -W -f '${Version}' "$1" 2>/dev/null || echo missing
}

candidate_of() {
    apt-cache policy "$1" 2>/dev/null | sed -n 's/^  Candidate: //p' | head -1
}

rp_enable() {
    rp_last_error=$("$TRAY" enable --policy "$POLICY" 2>&1 >/dev/null)
    rp_last_rc=$?
    return $rp_last_rc
}

rp_disable() {
    "$TRAY" disable --policy "$POLICY" >/dev/null 2>>"$LOG"
}

rp_join() {
    "$TRAY" join --policy "$POLICY" --owner "$OWNER_B" >/dev/null 2>>"$LOG"
}

rp_status_state() {
    "$TRAY" status --policy "$POLICY" 2>/dev/null |
        sed -n 's/.*state=\([a-z-]*\).*/\1/p' | head -1
}

rp_status_owners() {
    "$TRAY" status --policy "$POLICY" 2>/dev/null |
        sed -n 's/.*owners=\([^ ]*\).*/\1/p' | head -1
}

rp_status_module() {
    "$TRAY" status --policy "$POLICY" 2>/dev/null |
        sed -n 's/.*module=\([^ ]*\).*/\1/p' | head -1
}

case_journal_direct_cycle() {
    rp_enable
    rc=$?
    if [ $rc -ne 0 ] || [ ! -f "$JOURNAL" ] || [ -L "$JOURNAL" ]; then
        record journal-direct-cycle 0 "enable rc=$rc journal=$([ -f "$JOURNAL" ] && echo present || echo gone)"
        rp_disable
        return
    fi
    if [ -e "$JOURNAL_STAGE" ] || [ -L "$JOURNAL_STAGE" ]; then
        record journal-direct-cycle 0 "a completed enable left the recovery stage behind"
        rp_disable
        return
    fi
    rp_disable
    rc=$?
    if [ $rc -eq 0 ] && [ ! -e "$JOURNAL" ] && [ ! -L "$JOURNAL" ]         && [ ! -e "$JOURNAL_STAGE" ] && [ ! -L "$JOURNAL_STAGE" ]; then
        record journal-direct-cycle 1 "a full policy cycle leaves the exact canonical journal and the exact recovery stage absent under /var/lib"
    else
        record journal-direct-cycle 0 "disable rc=$rc journal=$([ -e "$JOURNAL" ] || [ -L "$JOURNAL" ] && echo present || echo gone) stage=$([ -e "$JOURNAL_STAGE" ] || [ -L "$JOURNAL_STAGE" ] && echo present || echo gone)"
    fi
}

case_journal_operator_neighbor() {
    operator=/var/lib/qol-wave2-operator-note
    if [ -e "$operator" ] || [ -L "$operator" ]; then
        record journal-operator-neighbor 0 "an unexpected pre-existing operator target exists at $operator; refusing to overwrite it"
        return
    fi
    printf 'operator bytes' > "$operator"
    rp_enable
    rc=$?
    if [ $rc -ne 0 ] || [ "$(rp_status_state)" != active ]; then
        record journal-operator-neighbor 0 "enable rc=$rc state=$(rp_status_state)"
        rm -f "$operator"
        return
    fi
    rp_disable
    rc=$?
    operator_now=$(cat "$operator" 2>/dev/null || echo gone)
    if [ $rc -eq 0 ] && [ "$operator_now" = "operator bytes" ]         && [ ! -e "$JOURNAL" ] && [ ! -L "$JOURNAL" ]         && [ ! -e "$JOURNAL_STAGE" ] && [ ! -L "$JOURNAL_STAGE" ]; then
        record journal-operator-neighbor 1 "an unrelated /var/lib entry survived the policy cycle byte for byte while both exact journal paths disappeared"
    else
        record journal-operator-neighbor 0 "disable rc=$rc operator=$operator_now"
    fi
    rm -f "$operator"
}

case_journal_valid_stage_recovery() {
    if [ -e "$JOURNAL" ] || [ -L "$JOURNAL" ]         || [ -e "$JOURNAL_STAGE" ] || [ -L "$JOURNAL_STAGE" ]; then
        record journal-valid-stage-recovery 0 "the crash-recovery case requires clean canonical and fixed-stage preconditions"
        return
    fi
    QOL_RESIDENT_CRASH_POINT=after-journal-stage-link "$TRAY" enable \
        --policy "$POLICY" >/dev/null 2>>"$LOG"
    rc=$?
    if [ $rc -eq 0 ] || [ ! -e "$JOURNAL_STAGE" ]; then
        record journal-valid-stage-recovery 0 "crash point did not abort after the journal stage link (rc=$rc stage=$([ -e "$JOURNAL_STAGE" ] && echo present || echo gone))"
        return
    fi
    if [ -e "$JOURNAL" ] && [ -f "$JOURNAL" ]; then
        record journal-valid-stage-recovery 0 "the first adoption must not have a canonical before the interrupted write; found one"
        rm -f "$JOURNAL_STAGE" 2>/dev/null || true
        return
    fi
    state=$(rp_status_state)
    if [ "$state" = absent ]; then
        record journal-valid-stage-recovery 0 "an existing recovery stage must be visible to status, never Absent"
        rm -f "$JOURNAL_STAGE" 2>/dev/null || true
        return
    fi
    rp_enable
    rc=$?
    if [ $rc -ne 0 ] || [ "$(rp_status_state)" != active ]         || [ -e "$JOURNAL_STAGE" ] || [ -L "$JOURNAL_STAGE" ]; then
        record journal-valid-stage-recovery 0 "recovery enable rc=$rc stage=$([ -e "$JOURNAL_STAGE" ] || [ -L "$JOURNAL_STAGE" ] && echo present || echo gone)"
        return
    fi
    rp_disable
    rc=$?
    if [ $rc -eq 0 ] && [ ! -e "$JOURNAL" ] && [ ! -L "$JOURNAL" ]         && [ ! -e "$JOURNAL_STAGE" ] && [ ! -L "$JOURNAL_STAGE" ]; then
        record journal-valid-stage-recovery 1 "a crash after the stage link is recovered by the next locked enable, the recovery disable succeeds, and both exact journal paths are absent at completion"
    else
        record journal-valid-stage-recovery 0 "recovery disable rc=$rc journal=$([ -e "$JOURNAL" ] || [ -L "$JOURNAL" ] && echo present || echo gone) stage=$([ -e "$JOURNAL_STAGE" ] || [ -L "$JOURNAL_STAGE" ] && echo present || echo gone)"
    fi
}

case_journal_fixed_stage_occupied() {
    if [ -e "$JOURNAL_STAGE" ] || [ -L "$JOURNAL_STAGE" ]; then
        record journal-stage-collision 0 "an unexpected pre-existing fixed stage exists; refusing to overwrite it"
        return
    fi
    operator_bytes='Package: operator
Pin: version 9.9
Pin-Priority: 1001
'
    printf '%b' "$operator_bytes" > "$JOURNAL_STAGE"
    snapshot="$STAGE/stage.snapshot"
    cp "$JOURNAL_STAGE" "$snapshot"
    rp_enable
    rc=$?
    stage_same=$([ -f "$JOURNAL_STAGE" ] && cmp -s "$JOURNAL_STAGE" "$snapshot" && echo yes || echo no)
    if [ $rc -ne 0 ] && [ "$stage_same" = yes ] && [ ! -e "$JOURNAL" ]; then
        record journal-stage-collision 1 "an operator entry at the fixed recovery stage fails the enable closed and is preserved byte for byte; the canonical was never created"
    else
        record journal-stage-collision 0 "rc=$rc stage=$stage_same canonical=$([ -e "$JOURNAL" ] && echo present || echo gone)"
    fi
    rm -f "$JOURNAL_STAGE" 2>/dev/null || true
    rp_disable
}

case_collision() {
    printf 'Package: operator-keep\nPin: version 9.9\nPin-Priority: 1001\n' > "$FRAGMENT"
    printf 'Package: operator-keep\nPin: version 9.9\nPin-Priority: 1001\n' > "$FRAGMENT.expected"
    rp_enable
    rc=$?
    if [ $rc -ne 0 ] && cmp -s "$FRAGMENT" "$FRAGMENT.expected"; then
        record collision-no-clobber 1 "production enable refused a pre-existing unjournaled path; operator file intact"
    else
        record collision-no-clobber 0 "enable rc=$rc; expected refusal and intact operator file"
    fi
    rm -f "$FRAGMENT" "$FRAGMENT.expected"

    fragment_line="# qol resident policy: nvidia driver version pin"
    printf '%s\n' "$fragment_line" > "$FRAGMENT"
    rp_enable
    rc=$?
    if [ $rc -ne 0 ] && [ "$(head -1 "$FRAGMENT")" = "$fragment_line" ]; then
        record collision-marker-preserved 1 "unjournaled marker path refused and preserved; never deleted"
    else
        record collision-marker-preserved 0 "enable rc=$rc; expected refusal and preserved marker file"
    fi
    rm -f "$FRAGMENT"
}

case_boundary_a() {
    QOL_RESIDENT_CRASH_POINT=after-journal "$TRAY" enable \
        --policy "$POLICY" >/dev/null 2>>"$LOG"
    rc=$?
    if [ $rc -eq 0 ] || [ ! -f "$JOURNAL" ]; then
        record interrupted-preparing-journal 0 "crash point did not abort after the Preparing journal (rc=$rc)"
        return
    fi
    state=$(rp_status_state)
    if [ "$state" = preparing ] && [ ! -f "$FRAGMENT" ]; then
        rp_enable
        rc=$?
        if [ $rc -eq 0 ] && [ "$(rp_status_state)" = active ]; then
            record interrupted-preparing-journal 1 "real abort after the durable Preparing journal: recovery resumed adoption to active"
        else
            record interrupted-preparing-journal 0 "recovery enable rc=$rc state=$(rp_status_state)"
        fi
    else
        record interrupted-preparing-journal 0 "expected preparing without fragment, got state=$state"
    fi
    rp_disable
}

case_boundary_b() {
    QOL_RESIDENT_CRASH_POINT=after-staged-write "$TRAY" enable \
        --policy "$POLICY" >/dev/null 2>>"$LOG"
    rc=$?
    if [ $rc -eq 0 ] || [ ! -f "$JOURNAL" ]; then
        record interrupted-staged-write 0 "crash point did not abort after the staged write (rc=$rc)"
        return
    fi
    state=$(rp_status_state)
    staged_left=$(ls /etc/apt/preferences.d/.90qol-nvidia-driver.pref.qol-stage-* 2>/dev/null | wc -l)
    if [ "$state" = preparing ] && [ ! -f "$FRAGMENT" ] && [ "$staged_left" = 0 ]; then
        rp_enable
        rc=$?
        staged_after=$(ls /etc/apt/preferences.d/.90qol-nvidia-driver.pref.qol-stage-* 2>/dev/null | wc -l)
        if [ $rc -eq 0 ] && [ "$(rp_status_state)" = active ] && [ "$staged_after" = 0 ]; then
            record interrupted-staged-write 1 "real abort during the unnamed staged write left no named residue; recovery re-staged and published"
        else
            record interrupted-staged-write 0 "recovery enable rc=$rc state=$(rp_status_state) staged=$staged_after"
        fi
    else
        record interrupted-staged-write 0 "expected preparing with no staged residue, got state=$state staged=$staged_left"
    fi
    rp_disable
}

case_boundary_link() {
    QOL_RESIDENT_CRASH_POINT=after-link "$TRAY" enable \
        --policy "$POLICY" >/dev/null 2>>"$LOG"
    rc=$?
    if [ $rc -eq 0 ] || [ ! -f "$JOURNAL" ]; then
        record interrupted-staged-link 0 "crash point did not abort after the staged link (rc=$rc)"
        return
    fi
    state=$(rp_status_state)
    staged_left=$(ls /etc/apt/preferences.d/.90qol-nvidia-driver.pref.qol-stage-* 2>/dev/null | wc -l)
    if [ "$state" = preparing ] && [ ! -f "$FRAGMENT" ] && [ "$staged_left" -ge 1 ]; then
        rp_enable
        rc=$?
        staged_after=$(ls /etc/apt/preferences.d/.90qol-nvidia-driver.pref.qol-stage-* 2>/dev/null | wc -l)
        if [ $rc -eq 0 ] && [ "$(rp_status_state)" = active ] && [ "$staged_after" = 0 ]; then
            record interrupted-staged-link 1 "real abort after the staged link: recovery published the complete staged bytes and left no residue"
        else
            record interrupted-staged-link 0 "recovery enable rc=$rc state=$(rp_status_state) staged=$staged_after"
        fi
    else
        record interrupted-staged-link 0 "expected preparing with a complete staged file, got state=$state staged=$staged_left"
    fi
    rp_disable
}

case_boundary_publish() {
    QOL_RESIDENT_CRASH_POINT=after-publish "$TRAY" enable \
        --policy "$POLICY" >/dev/null 2>>"$LOG"
    rc=$?
    if [ $rc -eq 0 ] || [ ! -f "$JOURNAL" ]; then
        record interrupted-publish 0 "crash point did not abort after the publish (rc=$rc)"
        return
    fi
    state=$(rp_status_state)
    if [ "$state" = preparing ] && [ -f "$FRAGMENT" ]; then
        rp_enable
        rc=$?
        if [ $rc -eq 0 ] && [ "$(rp_status_state)" = active ]; then
            record interrupted-publish 1 "real abort after the no-replace publish: recovery finalized to active"
        else
            record interrupted-publish 0 "recovery enable rc=$rc state=$(rp_status_state)"
        fi
    else
        record interrupted-publish 0 "expected preparing with a published fragment, got state=$state"
    fi
    rp_disable
}

case_boundary_c() {
    rp_enable
    QOL_RESIDENT_CRASH_POINT=after-fragment-removal "$TRAY" disable \
        --policy "$POLICY" >/dev/null 2>>"$LOG"
    rc=$?
    if [ $rc -eq 0 ]; then
        record interrupted-release 0 "crash point did not abort during release"
        return
    fi
    state=$(rp_status_state)
    if [ "$state" = releasing ] && [ ! -f "$FRAGMENT" ]; then
        rp_disable
        rc=$?
        if [ $rc -eq 0 ] && [ ! -f "$JOURNAL" ] && [ "$(rp_status_state)" = absent ]; then
            record interrupted-release 1 "real abort during release: recovery removed the journal to Absent"
        else
            record interrupted-release 0 "recovery disable rc=$rc state=$(rp_status_state)"
        fi
    else
        record interrupted-release 0 "expected releasing, got state=$state"
    fi
}

case_fail_next_publish_fsync() {
    QOL_RESIDENT_FAIL_NEXT=publish-fsync "$TRAY" enable         --policy "$POLICY" >/dev/null 2>>"$LOG"
    rc=$?
    state=$(rp_status_state)
    staged_left=$(ls /etc/apt/preferences.d/.90qol-nvidia-driver.pref.qol-stage-* 2>/dev/null | wc -l)
    if [ $rc -ne 0 ] && [ "$state" = absent ] && [ ! -f "$FRAGMENT" ]         && [ ! -f "$JOURNAL" ] && [ "$staged_left" = 0 ]; then
        rp_enable
        rc=$?
        if [ $rc -eq 0 ] && [ "$(rp_status_state)" = active ]; then
            record publish-fsync-unwind 1 "a publish fsync failure unwound fragment, staged file, and journal to Absent; a normal retry then adopted"
        else
            record publish-fsync-unwind 0 "retry rc=$rc state=$(rp_status_state)"
        fi
    else
        record publish-fsync-unwind 0 "enable rc=$rc state=$state staged=$staged_left"
    fi
    rp_disable
}

case_fail_next_release_fsync() {
    rp_enable
    QOL_RESIDENT_FAIL_NEXT=release-fsync "$TRAY" disable         --policy "$POLICY" >/dev/null 2>>"$LOG"
    rc=$?
    state=$(rp_status_state)
    if [ $rc -ne 0 ] && [ "$state" = release-failed ] && [ -f "$JOURNAL" ] && [ ! -f "$FRAGMENT" ]; then
        rp_disable
        rc=$?
        if [ $rc -eq 0 ] && [ "$(rp_status_state)" = absent ]             && [ ! -f "$JOURNAL" ] && [ ! -f "$FRAGMENT" ]; then
            record release-fsync-evidence 1 "a release fsync failure kept ReleaseFailed evidence after fragment removal; the retry reached Absent"
        else
            record release-fsync-evidence 0 "retry rc=$rc state=$(rp_status_state)"
        fi
    else
        record release-fsync-evidence 0 "disable rc=$rc state=$state journal=$([ -f "$JOURNAL" ] && echo present || echo gone) fragment=$([ -f "$FRAGMENT" ] && echo present || echo gone)"
    fi
}

case_exact_copy_staged_collision() {
    QOL_RESIDENT_CRASH_POINT=after-link "$TRAY" enable \
        --policy "$POLICY" >/dev/null 2>>"$LOG"
    rc=$?
    if [ $rc -eq 0 ] || [ ! -f "$JOURNAL" ]; then
        record exact-copy-staged-preserved 0 "crash point did not abort after the staged link (rc=$rc)"
        return
    fi
    staged=$(grep -o '"staged_path":"[^"]*"' "$JOURNAL" | head -1 |
        sed 's/"staged_path":"//;s/"$//')
    if [ ! -f "$staged" ]; then
        record exact-copy-staged-preserved 0 "no staged file at $staged"
        return
    fi
    old_identity=$(stat -c '%d:%i' "$staged")
    snapshot="$staged.snapshot"
    cp "$staged" "$snapshot"
    replacement="$staged.replacement"
    cp "$staged" "$replacement"
    mv "$replacement" "$staged"
    new_identity=$(stat -c '%d:%i' "$staged")
    if [ "$old_identity" = "$new_identity" ] || ! cmp -s "$staged" "$snapshot"; then
        record exact-copy-staged-preserved 0 "replacement old=$old_identity new=$new_identity bytes=$([ cmp -s "$staged" "$snapshot" ] && echo same || echo different)"
        rm -f "$staged" "$snapshot"
        return
    fi
    rp_enable
    rc=$?
    state=$(rp_status_state)
    if [ $rc -ne 0 ] && cmp -s "$staged" "$snapshot" && [ "$state" = release-failed ] \
        && [ -f "$JOURNAL" ] && [ ! -f "$FRAGMENT" ]; then
        rm -f "$staged" "$snapshot"
        rp_disable
        rc=$?
        if [ $rc -eq 0 ] && [ "$(rp_status_state)" = absent ] \
            && [ ! -f "$JOURNAL" ] && [ ! -f "$FRAGMENT" ]; then
            rp_enable
            rc=$?
            if [ $rc -eq 0 ] && [ "$(rp_status_state)" = active ]; then
                rp_disable
                rc=$?
                if [ $rc -eq 0 ] && [ "$(rp_status_state)" = absent ] \
                    && [ ! -f "$JOURNAL" ] && [ ! -f "$FRAGMENT" ]; then
                    record exact-copy-staged-preserved 1 "a byte-identical staged replacement (old inode $old_identity, new inode $new_identity) was preserved and failed closed to ReleaseFailed with journal evidence and no fragment; removing it retired the journal to Absent, a fresh enable adopted, and the final release reached Absent with no journal or fragment"
                else
                    record exact-copy-staged-preserved 0 "final disable rc=$rc state=$(rp_status_state)"
                fi
            else
                record exact-copy-staged-preserved 0 "fresh enable rc=$rc state=$(rp_status_state)"
                rp_disable
            fi
        else
            record exact-copy-staged-preserved 0 "retire rc=$rc state=$(rp_status_state)"
        fi
    else
        record exact-copy-staged-preserved 0 "rc=$rc preserved=$([ cmp -s "$staged" "$snapshot" ] && echo same || echo different) state=$state"
        rm -f "$staged" "$snapshot"
        rp_disable
    fi
}

case_journal_wrong_inode_stage_preserved() {
    rp_enable
    rc=$?
    if [ $rc -ne 0 ] || [ ! -f "$JOURNAL" ]; then
        record journal-stage-wrong-inode 0 "enable rc=$rc journal=$([ -f "$JOURNAL" ] && echo present || echo gone)"
        rp_disable
        return
    fi
    cp "$JOURNAL" "$JOURNAL_STAGE"
    snapshot="$STAGE/stage-copy.snapshot"
    cp "$JOURNAL_STAGE" "$snapshot"
    rp_enable
    rc=$?
    stage_same=$([ -f "$JOURNAL_STAGE" ] && cmp -s "$JOURNAL_STAGE" "$snapshot" && echo yes || echo no)
    if [ $rc -ne 0 ] && [ "$stage_same" = yes ]; then
        record journal-stage-wrong-inode 1 "a byte-copied journal at the fixed recovery stage has the wrong inode; the locked enable fails closed and preserves it byte for byte"
    else
        record journal-stage-wrong-inode 0 "rc=$rc stage=$stage_same"
    fi
    rm -f "$JOURNAL_STAGE" 2>/dev/null || true
    rp_disable
}

case_dangling_fragment_residue() {
    state_before=$(rp_status_state)
    if [ "$state_before" != absent ]; then
        record dangling-fragment-unjournaled 0 "expected absent, got $state_before"
        return
    fi
    ln -s /var/tmp/w2-missing-target "$FRAGMENT"
    state=$(rp_status_state)
    rp_enable
    rc=$?
    if [ "$state" = unjournaled ] && [ $rc -ne 0 ] && [ -L "$FRAGMENT" ] && [ ! -f "$JOURNAL" ]; then
        record dangling-fragment-unjournaled 1 "a dangling fragment symlink is Unjournaled, preserved, and never reported Absent; enable refused it"
    else
        record dangling-fragment-unjournaled 0 "state=$state rc=$rc symlink=$([ -L "$FRAGMENT" ] && echo kept || echo gone)"
    fi
    rm -f "$FRAGMENT"
}

case_nofollow_dir_swap() {
    prefs=/etc/apt/preferences.d
    real="$STAGE/prefs-real"
    evil="$STAGE/prefs-evil"
    if [ -e "$real" ] || [ -e "$evil" ]; then
        record nofollow-dir-swap-refused 0 "fixture paths already exist"
        return
    fi
    mv "$prefs" "$real" || {
        record nofollow-dir-swap-refused 0 "fixture move failed"
        return
    }
    mkdir -p "$evil"
    ln -s "$evil" "$prefs"
    rp_enable
    rc=$?
    evil_hits=$(ls -A "$evil" 2>/dev/null | grep -c '90qol' || true)
    journal_gone=$([ -e "$JOURNAL" ] || [ -L "$JOURNAL" ] && echo no || echo yes)
    rm -f "$prefs"
    mv "$real" "$prefs" || {
        record nofollow-dir-swap-refused 0 "directory restore failed"
        return
    }
    rp_enable
    rc2=$?
    if [ $rc -ne 0 ] && [ "$evil_hits" = 0 ] && [ "$journal_gone" = yes ]         && [ $rc2 -eq 0 ] && [ "$(rp_status_state)" = active ]; then
        record nofollow-dir-swap-refused 1 "a symlink-swapped preferences directory was refused without touching its target; the real directory was restored and adoption works"
    else
        record nofollow-dir-swap-refused 0 "enable rc=$rc evil=$evil_hits journal=$journal_gone restore rc=$rc2 state=$(rp_status_state)"
    fi
    rp_disable
}

case_adopt_pins() {
    rp_enable
    rc=$?
    a=$(candidate_of nvidia-driver-fixture-a)
    b=$(candidate_of nvidia-driver-fixture-b)
    c=$(candidate_of fixture-ctl)
    if [ $rc -eq 0 ] && [ "$(rp_status_state)" = active ] && [ "$a" = 1.0 ] && [ "$b" = 1.0 ] && [ "$c" != 1.0 ]; then
        record adopt-pins 1 "production adoption active; protected candidates pinned at 1.0; control candidate $c"
    else
        record adopt-pins 0 "enable rc=$rc error=$rp_last_error state=$(rp_status_state) candidates a=$a b=$b ctl=$c"
    fi
}

case_module_version_snapshotted() {
    module=$(rp_status_module)
    a=$(version_of nvidia-driver-fixture-a)
    if [ "$module" = 580.159.02 ] && [ "$a" = 1.0 ]; then
        record module-version-snapshotted 1 "expected module version 580.159.02 comes from the module probe, not the 1.0 fixture package version"
    else
        record module-version-snapshotted 0 "module=$module package-version=$a; expected 580.159.02 vs 1.0"
    fi
}

case_status_as_user() {
    state=$(runuser -u nobody -- "$TRAY" status --policy "$POLICY" 2>/dev/null |
        sed -n 's/.*state=\([a-z-]*\).*/\1/p' | head -1)
    if [ "$state" = active ]; then
        record status-as-user 1 "the read-only policy intent probe works without root: state=$state"
    else
        record status-as-user 0 "unprivileged status probe state=$state; expected active"
    fi
}

case_upgrade() {
    apt-get upgrade -y >/dev/null 2>>"$LOG"
    rc=$?
    a=$(version_of nvidia-driver-fixture-a)
    b=$(version_of nvidia-driver-fixture-b)
    c=$(version_of fixture-ctl)
    if [ $rc -eq 0 ] && [ "$a" = 1.0 ] && [ "$b" = 1.0 ] && [ "$c" = 4.0 ]; then
        record upgrade-pinned 1 "ordinary upgrade: protected stayed 1.0, control reached newest 4.0"
    else
        record upgrade-pinned 0 "upgrade rc=$rc versions a=$a b=$b ctl=$c"
    fi
}

case_full_upgrade() {
    apt-get full-upgrade -y >/dev/null 2>>"$LOG"
    rc=$?
    a=$(version_of nvidia-driver-fixture-a)
    b=$(version_of nvidia-driver-fixture-b)
    c=$(version_of fixture-ctl)
    if [ $rc -eq 0 ] && [ "$a" = 1.0 ] && [ "$b" = 1.0 ] && [ "$c" = 4.0 ]; then
        record full-upgrade-pinned 1 "full-upgrade: protected stayed 1.0, control at newest 4.0"
    else
        record full-upgrade-pinned 0 "full-upgrade rc=$rc versions a=$a b=$b ctl=$c"
    fi
}

case_unattended() {
    build_deb "$CONTROL" 5.0
    gen_packages
    gen_release
    apt-get update -qq >/dev/null 2>>"$LOG" || fail_hard "unattended prep update"
    mode=apt-get-mirror
    pattern=
    rc=1
    if command -v unattended-upgrade >/dev/null 2>&1; then
        mode=unattended-upgrades
        pattern='origin=*'
        printf 'Unattended-Upgrade::Origins-Pattern {\n    "%s";\n};\n' "$pattern" \
            > /etc/apt/apt.conf.d/90wave2-unattended
        unattended-upgrade -d >/dev/null 2>>"$LOG"
        rc=$?
        rm -f /etc/apt/apt.conf.d/90wave2-unattended
    else
        apt-get -o Dpkg::Options::=--force-confold -o Dpkg::Options::=--force-confdef \
            -y upgrade >/dev/null 2>>"$LOG"
        rc=$?
    fi
    a=$(version_of nvidia-driver-fixture-a)
    b=$(version_of nvidia-driver-fixture-b)
    c=$(version_of fixture-ctl)
    evidence=
    if [ "$mode" = unattended-upgrades ] && [ $rc -ne 0 ]; then
        if command -v tail >/dev/null 2>&1; then
            uu_log=/var/log/unattended-upgrades/unattended-upgrades.log
            if [ -f "$uu_log" ] && [ -r "$uu_log" ]; then
                evidence=$(tail -n 1 "$uu_log" 2>/dev/null || true)
            fi
            if [ -z "$evidence" ] && [ -f "$LOG" ] && [ -r "$LOG" ]; then
                evidence=$(tail -n 1 "$LOG" 2>/dev/null || true)
            fi
        fi
    fi
    if [ $rc -eq 0 ] && [ "$a" = 1.0 ] && [ "$b" = 1.0 ] && [ "$c" = 5.0 ]; then
        record unattended-pinned 1 "unattended run (mode=$mode, exit=0): fresh control moved to 5.0, protected stayed 1.0"
    else
        record unattended-pinned 0 "unattended mode=$mode pattern=$pattern exit=$rc evidence=$evidence versions a=$a b=$b ctl=$c"
    fi
}

case_control_advances() {
    c=$(version_of fixture-ctl)
    if [ "$c" = 5.0 ]; then
        record control-advances 1 "unprotected control reached the newest repo version 5.0"
    else
        record control-advances 0 "control stopped at $c; expected 5.0"
    fi
}

case_drift_and_release() {
    echo "# operator note: pin me harder" >> "$FRAGMENT"
    state=$(rp_status_state)
    if [ "$state" = drifted ]; then
        record drift-observed 1 "operator modification observed as drifted"
    else
        record drift-observed 0 "expected drifted, got $state"
    fi
    rp_disable
    rc=$?
    if [ $rc -ne 0 ] && [ "$(rp_status_state)" = release-failed ] && [ -f "$FRAGMENT" ] && grep -q "operator note" "$FRAGMENT"; then
        record drift-release-no-deletion 1 "release refused drifted fragment; release-failed state preserved evidence and operator modification"
    else
        record drift-release-no-deletion 0 "disable rc=$rc state=$(rp_status_state) fragment_present=$([ -f "$FRAGMENT" ] && echo yes || echo no)"
    fi
    rm -f "$FRAGMENT"
    rp_disable
    rc=$?
    if [ $rc -eq 0 ] && [ ! -f "$JOURNAL" ] && [ ! -f "$FRAGMENT" ]; then
        record release 1 "release with restored identity removed fragment and journal"
    else
        record release 0 "disable rc=$rc journal=$([ -f "$JOURNAL" ] && echo present || echo gone)"
    fi
}

case_ownership() {
    rp_enable
    owners_first=$(rp_status_owners)
    rp_join
    rc_join=$?
    owners_two=$(rp_status_owners)
    if [ $rc_join -eq 0 ] && [ "$owners_two" != "$owners_first" ] && [ -f "$FRAGMENT" ]; then
        rp_disable
        rc1=$?
        owners_after_first=$(rp_status_owners)
        if [ $rc1 -eq 0 ] && [ "$owners_after_first" = "$OWNER_B" ] && [ -f "$JOURNAL" ] && [ -f "$FRAGMENT" ]; then
            "$TRAY" disable --policy "$POLICY" --owner "$OWNER_B" >/dev/null 2>>"$LOG"
            rc2=$?
            if [ $rc2 -eq 0 ] && [ ! -f "$JOURNAL" ] && [ ! -f "$FRAGMENT" ]; then
                record ownership 1 "second owner joined without mutation; releasing owners in order restored only at the last release"
            else
                record ownership 0 "last release rc=$rc2"
            fi
        else
            record ownership 0 "first release rc=$rc1 owners=$owners_after_first"
        fi
    else
        record ownership 0 "join rc=$rc_join owners first=$owners_first two=$owners_two"
    fi
}

case_owner_release_order() {
    rp_enable
    rp_join
    "$TRAY" disable --policy "$POLICY" --owner "$OWNER_B" >/dev/null 2>>"$LOG"
    rc1=$?
    owners_after_b=$(rp_status_owners)
    if [ $rc1 -eq 0 ] && [ "$owners_after_b" != "$OWNER_B" ] && [ -f "$JOURNAL" ] && [ -f "$FRAGMENT" ]; then
        rp_disable
        rc2=$?
        if [ $rc2 -eq 0 ] && [ ! -f "$JOURNAL" ] && [ ! -f "$FRAGMENT" ]; then
            record owner-release-order 1 "arbitrary release order (second owner first) left the policy owned, then restored at the last release"
        else
            record owner-release-order 0 "last release rc=$rc2"
        fi
    else
        record owner-release-order 0 "second-owner release rc=$rc1 owners=$owners_after_b"
    fi
}

case_owner_idempotence() {
    rp_enable
    owners_before=$(rp_status_owners)
    rp_join
    rp_join
    owners_after_joins=$(rp_status_owners)
    rp_disable
    rp_disable
    rc_double=$?
    owners_after_double=$(rp_status_owners)
    state_after_double=$(rp_status_state)
    "$TRAY" disable --policy "$POLICY" --owner "$OWNER_B" >/dev/null 2>>"$LOG"
    rc_b=$?
    state_final=$(rp_status_state)
    if [ "$owners_after_joins" = "$owners_before,$OWNER_B" ] && [ $rc_double -eq 0 ] \
        && [ "$owners_after_double" = "$OWNER_B" ] && [ "$state_after_double" = active ] \
        && [ $rc_b -eq 0 ] && [ "$state_final" = absent ]; then
        record owner-idempotence 1 "join and release are idempotent; the last owner release left Absent"
    else
        record owner-idempotence 0 "owners before=$owners_before after=$owners_after_joins double rc=$rc_double owners=$owners_after_double state=$state_after_double b rc=$rc_b final=$state_final"
    fi
}

case_cross_process_lock() {
    rp_enable >/dev/null 2>>"$LOG" &
    bg=$!
    rp_enable
    rc_fg=$?
    wait $bg
    rc_bg=$?
    state=$(rp_status_state)
    if [ $rc_fg -eq 0 ] && [ $rc_bg -eq 0 ] && [ "$state" = active ]; then
        record cross-process-lock 1 "concurrent enable processes serialized on the residue-free lock (fg=$rc_fg bg=$rc_bg)"
    else
        record cross-process-lock 0 "fg=$rc_fg bg=$rc_bg state=$state"
    fi
    rp_disable
}

case_update_continuity() {
    rp_enable
    owners_first=$(rp_status_owners)
    rp_disable
    rp_enable
    owners_second=$(rp_status_owners)
    if [ -n "$owners_first" ] && [ "$owners_first" = "$owners_second" ]; then
        record update-continuity 1 "the residency owner is stable across install cycles (machine-derived, not an install id)"
    else
        record update-continuity 0 "owner first=$owners_first second=$owners_second"
    fi
    rp_disable
}

case_deb_setup() {
    echo "w2: installing the guest-only headless dependency provider, then the production deb with a plain dpkg -i (no force options)"
    install_headless_deps_provider
    dpkg -i "$PAYLOAD_ROOT/qol-tray.deb" >/dev/null 2>>"$LOG" || fail_hard "qol-tray deb install"
    require_apt_health
    dpkg-deb -x "$PAYLOAD_ROOT/qol-tray.deb" "$STAGE/debextract" 2>/dev/null || fail_hard "deb extract"
    unresolved=$(adapter_unresolved_libs)
    payload_ok=1
    [ -x /usr/bin/qol-tray ] || payload_ok=0
    [ -x /usr/lib/qol-tray/qol-resident-policy ] || payload_ok=0
    if ! "$TRAY" status >/dev/null 2>>"$LOG"; then payload_ok=0; fi
    [ -f /var/lib/dpkg/info/qol-tray.prerm ] || payload_ok=0
    [ -f /var/lib/dpkg/info/qol-tray.postrm ] || payload_ok=0
    [ -f /var/lib/dpkg/info/qol-tray.postinst ] || payload_ok=0
    [ -f /etc/xdg/autostart/qol-tray.desktop ] || payload_ok=0
    dpkg -S /etc/xdg/autostart/qol-tray.desktop 2>/dev/null | grep -q '^qol-tray: ' || payload_ok=0
    version=$(dpkg-query -W -f '${Version}' qol-tray 2>/dev/null)
    if [ "$payload_ok" = 1 ] && [ -n "$version" ] && [ "$unresolved" = 0 ]; then
        record deb-setup 1 "the production deb installed with a plain dpkg -i after the guest-only provider (version $version); apt-get check passed, dpkg --audit empty, and the extracted resident adapter has no unresolved dynamic libraries"
    else
        record deb-setup 0 "payload_ok=$payload_ok version=$version unresolved=$unresolved"
    fi
}

case_interrupted_reboot() {
    QOL_RESIDENT_CRASH_POINT=after-journal "$TRAY" enable \
        --policy "$POLICY" >/dev/null 2>>"$LOG"
    rc=$?
    if [ $rc -eq 0 ] || [ ! -f "$JOURNAL" ]; then
        record interrupted-reboot 0 "crash point did not abort before the reboot (rc=$rc)"
        return
    fi
    record interrupted-reboot 1 "adoption left a Preparing journal before the reboot"
}

case_reboot_resume() {
    state=$(rp_status_state)
    if [ "$state" != preparing ]; then
        record reboot-resume 0 "expected preparing after the reboot, got $state"
        return
    fi
    rp_enable
    rc=$?
    if [ $rc -eq 0 ] && [ "$(rp_status_state)" = active ]; then
        record reboot-resume 1 "adoption resumed to active across the real reboot"
    else
        record reboot-resume 0 "resume rc=$rc state=$(rp_status_state)"
    fi
    rp_disable
}

case_staged_fault_recovery() {
    QOL_RESIDENT_CRASH_POINT=after-journal "$TRAY" enable \
        --policy "$POLICY" >/dev/null 2>>"$LOG"
    rc=$?
    if [ $rc -eq 0 ] || [ ! -f "$JOURNAL" ]; then
        record staged-fault-recovery 0 "crash point did not abort (rc=$rc)"
        return
    fi
    staged=$(grep -o '"staged_path":"[^"]*"' "$JOURNAL" | head -1 |
        sed 's/"staged_path":"//;s/"$//')
    printf 'operator bytes' > "$staged" 2>/dev/null || true
    rp_enable
    rc=$?
    preserved=$(cat "$staged" 2>/dev/null || echo gone)
    state=$(rp_status_state)
    if [ $rc -ne 0 ] && [ "$preserved" = "operator bytes" ] \
        && [ "$state" = release-failed ] && [ -f "$JOURNAL" ] && [ ! -f "$FRAGMENT" ]; then
        rm -f "$staged"
        rp_disable
        rc=$?
        if [ $rc -eq 0 ] && [ "$(rp_status_state)" = absent ] \
            && [ ! -f "$JOURNAL" ] && [ ! -f "$FRAGMENT" ]; then
            rp_enable
            rc=$?
            if [ $rc -eq 0 ] && [ "$(rp_status_state)" = active ]; then
                record staged-fault-recovery 1 "an unprovable staged-path file was preserved byte for byte and failed closed to ReleaseFailed with journal evidence and no fragment; removing it retired the journal to Absent and a fresh enable adopted"
            else
                record staged-fault-recovery 0 "fresh enable rc=$rc state=$(rp_status_state)"
            fi
        else
            record staged-fault-recovery 0 "retire rc=$rc state=$(rp_status_state)"
        fi
    else
        record staged-fault-recovery 0 "enable rc=$rc preserved=$preserved state=$state journal=$([ -f "$JOURNAL" ] && echo present || echo gone)"
        rm -f "$staged"
        rp_disable
    fi
}

case_raw_artifact_gate() {
    "$PAYLOAD_ROOT/qol-resident-policy-plain" enable --policy "$POLICY" >/dev/null 2>>"$LOG"
    rc=$?
    state=$(rp_status_state)
    if [ $rc -ne 0 ] && [ "$state" = absent ]; then
        record raw-artifact-gate 1 "a raw non-sandbox adapter refused activation; host unchanged"
    else
        record raw-artifact-gate 0 "rc=$rc state=$state"
    fi
}

install_sandbox_adapter() {
    cp "$PAYLOAD_ROOT/qol-resident-policy" /usr/lib/qol-tray/qol-resident-policy || fail_hard "sandbox adapter swap"
    chmod 755 /usr/lib/qol-tray/qol-resident-policy
    export QOL_RESIDENT_SANDBOX_CARRIER=1
}

restore_production_adapter() {
    if [ ! -f "$STAGE/debextract/usr/lib/qol-tray/qol-resident-policy" ]; then
        dpkg-deb -x "$PAYLOAD_ROOT/qol-tray.deb" "$STAGE/debextract" 2>/dev/null || fail_hard "deb extract"
    fi
    cp "$STAGE/debextract/usr/lib/qol-tray/qol-resident-policy" /usr/lib/qol-tray/qol-resident-policy ||
        fail_hard "production adapter restore"
    chmod 755 /usr/lib/qol-tray/qol-resident-policy
    unset QOL_RESIDENT_SANDBOX_CARRIER
}

case_contract_fixture() {
    mkdir -p "$REPO/pool"
    build_deb nvidia-driver-fixture-a 1.0
    dpkg -i "$REPO/pool/nvidia-driver-fixture-a_1.0_all.deb" >/dev/null 2>>"$LOG" ||
        fail_hard "contract fixture install"
    if [ "$(version_of nvidia-driver-fixture-a)" = 1.0 ]; then
        record contract-fixture 1 "a harmless fixture inside the fixed NVIDIA driver family is installed before production activation"
    else
        record contract-fixture 0 "fixture version=$(version_of nvidia-driver-fixture-a)"
    fi
}

case_package_contract() {
    echo "w2: installing the guest-only headless dependency provider, then the production deb with a plain dpkg -i (no force options)"
    install_headless_deps_provider
    dpkg -i "$PAYLOAD_ROOT/qol-tray.deb" >/dev/null 2>>"$LOG" || fail_hard "production deb install"
    require_apt_health
    dpkg-deb -x "$PAYLOAD_ROOT/qol-tray.deb" "$STAGE/debextract" 2>/dev/null || fail_hard "deb extract"
    unresolved=$(adapter_unresolved_libs)
    payload_ok=1
    [ -x /usr/bin/qol-tray ] || payload_ok=0
    [ -x /usr/lib/qol-tray/qol-resident-policy ] || payload_ok=0
    [ -f /var/lib/dpkg/info/qol-tray.prerm ] || payload_ok=0
    [ -f /var/lib/dpkg/info/qol-tray.postrm ] || payload_ok=0
    [ -f /var/lib/dpkg/info/qol-tray.postinst ] || payload_ok=0
    "$TRAY" status >/dev/null 2>>"$LOG" || payload_ok=0
    [ -f /etc/xdg/autostart/qol-tray.desktop ] || payload_ok=0
    [ -f /usr/share/applications/qol-tray.desktop ] || payload_ok=0
    dpkg -S /etc/xdg/autostart/qol-tray.desktop 2>/dev/null | grep -q '^qol-tray: ' || payload_ok=0
    cmp -s /etc/xdg/autostart/qol-tray.desktop "$STAGE/debextract/etc/xdg/autostart/qol-tray.desktop" || payload_ok=0
    grep -Eq '/home|/root' "$STAGE/debextract/DEBIAN/postinst" && payload_ok=0
    home_hits=$(find /home -iname '*qol*' 2>/dev/null | head -1)
    [ -z "$home_hits" ] || payload_ok=0
    version=$(dpkg-query -W -f '${Version}' qol-tray 2>/dev/null)
    if [ "$payload_ok" = 1 ] && [ -n "$version" ] && [ "$unresolved" = 0 ]; then
        record package-contract 1 "the real deb installed offline with a plain dpkg -i after the guest-only provider (version $version); apt-get check passed, dpkg --audit empty, the extracted resident adapter resolves all dynamic libraries, the autostart desktop file is dpkg-owned at /etc/xdg/autostart and /usr/share/applications, and postinst writes nothing under /home or /root"
    else
        record package-contract 0 "payload=$payload_ok version=$version unresolved=$unresolved"
    fi
}

case_package_contract_active() {
    rp_enable
    rc=$?
    if [ $rc -eq 0 ] && [ "$(rp_status_state)" = active ]; then
        record package-contract-active 1 "the production adapter adopted the policy to Active through the Debian carrier proof"
    else
        record package-contract-active 0 "enable rc=$rc state=$(rp_status_state) error=$rp_last_error"
    fi
}

case_contract_fail_closed() {
    "$TRAY" join --policy "$POLICY" --owner "$OWNER_B" >/dev/null 2>>"$LOG" || {
        record contract-fail-closed 0 "join failed"
        return
    }
    dpkg -r qol-tray >/dev/null 2>>"$LOG"
    rc=$?
    if [ $rc -ne 0 ] && [ -e "$JOURNAL" ] && [ -f "$FRAGMENT" ]; then
        "$TRAY" disable --policy "$POLICY" --owner "$OWNER_B" >/dev/null 2>>"$LOG"
        rc2=$?
        dpkg -r qol-tray >/dev/null 2>>"$LOG"
        rc3=$?
        if [ $rc2 -eq 0 ] && [ $rc3 -eq 0 ] && [ ! -e "$JOURNAL" ] && [ ! -L "$JOURNAL" ] \
            && [ ! -e "$FRAGMENT" ] && [ ! -L "$FRAGMENT" ]; then
            record contract-fail-closed 1 "a second owner made dpkg removal fail closed; releasing the remaining owner permitted clean removal"
        else
            record contract-fail-closed 0 "release rc=$rc2 remove rc=$rc3"
        fi
    else
        record contract-fail-closed 0 "dpkg -r rc=$rc journal=$([ -e "$JOURNAL" ] || [ -L "$JOURNAL" ] && echo present || echo gone)"
    fi
}

case_contract_lifecycle() {
    dpkg -r qol-tray >/dev/null 2>>"$LOG"
    rc_remove=$?
    if [ $rc_remove -eq 0 ] && [ ! -e "$JOURNAL" ] && [ ! -L "$JOURNAL" ]         && [ ! -e "$FRAGMENT" ] && [ ! -L "$FRAGMENT" ]; then
        dpkg -P qol-tray >/dev/null 2>>"$LOG"
        rc_purge=$?
        dpkg -P "$PROVIDER" >/dev/null 2>>"$LOG"
        rc_provider=$?
        provider_gone=$(dpkg-query -W -f '${Status}' "$PROVIDER" 2>/dev/null || echo gone)
        if [ $rc_purge -eq 0 ] && [ ! -e "$JOURNAL" ] && [ ! -L "$JOURNAL" ]             && [ ! -e "$FRAGMENT" ] && [ ! -L "$FRAGMENT" ]             && [ $rc_provider -eq 0 ] && [ "$provider_gone" = gone ]; then
            record contract-lifecycle 1 "the shipped prerm release hook restored Absent on dpkg -r, the purge left no journal or fragment, and the guest-only provider fixture was removed after qol-tray"
        else
            record contract-lifecycle 0 "purge rc=$rc_purge provider rc=$rc_provider status=$provider_gone"
        fi
    else
        record contract-lifecycle 0 "remove rc=$rc_remove journal=$([ -e "$JOURNAL" ] || [ -L "$JOURNAL" ] && echo present || echo gone) fragment=$([ -e "$FRAGMENT" ] || [ -L "$FRAGMENT" ] && echo present || echo gone)"
    fi
}

case_deb_lifecycle() {
    state_before=$(rp_status_state)
    if [ "$state_before" != active ]; then
        record deb-lifecycle 0 "expected an active policy before the package lifecycle, got $state_before"
        return
    fi
    owners_before=$(rp_status_owners)
    dpkg -i "$PAYLOAD_ROOT/qol-tray.deb" >/dev/null 2>>"$LOG" || {
        record deb-lifecycle 0 "production deb reinstall failed"
        return
    }
    require_apt_health
    state_after=$(rp_status_state)
    owners_after=$(rp_status_owners)
    version=$(dpkg-query -W -f '${Version}' qol-tray 2>/dev/null)
    if [ "$state_after" = active ] && [ "$owners_before" = "$owners_after" ] && [ -n "$version" ]; then
        remove_evidence=$STAGE/deb-remove.log
        dpkg -r qol-tray >"$remove_evidence" 2>&1
        rc_remove=$?
        if [ $rc_remove -eq 0 ] && [ ! -e "$JOURNAL" ] && [ ! -L "$JOURNAL" ]             && [ ! -e "$FRAGMENT" ] && [ ! -L "$FRAGMENT" ]; then
            rm -f "$remove_evidence"
            rc_cleanup=$?
            if [ $rc_cleanup -ne 0 ] || [ -e "$remove_evidence" ] || [ -L "$remove_evidence" ]; then
                record deb-lifecycle 0 "remove evidence cleanup failed rc=$rc_cleanup"
                return
            fi
            dpkg -P qol-tray >/dev/null 2>>"$LOG"
            rc_purge=$?
            if [ $rc_purge -eq 0 ] && [ ! -e "$JOURNAL" ] && [ ! -L "$JOURNAL" ]                 && [ ! -e "$FRAGMENT" ] && [ ! -L "$FRAGMENT" ]; then
                dpkg -P "$PROVIDER" >/dev/null 2>>"$LOG"
                rc_provider=$?
                provider_gone=$(dpkg-query -W -f '${Status}' "$PROVIDER" 2>/dev/null || echo gone)
                if [ $rc_provider -eq 0 ] && [ "$provider_gone" = gone ]; then
                    record deb-lifecycle 1 "the production deb reinstall preserved the active stable owner ($owners_before); dpkg remove ran the shipped release hook to Absent with no journal or fragment, and the guest-only provider fixture was removed after qol-tray"
                else
                    record deb-lifecycle 0 "purge rc=$rc_purge provider rc=$rc_provider status=$provider_gone"
                fi
            else
                record deb-lifecycle 0 "purge rc=$rc_purge"
            fi
        else
            remove_cause=$(sed -n '/resident-policy:/p' "$remove_evidence" 2>/dev/null | tail -n 1)
            if [ -n "$remove_cause" ]; then
                remove_cause=$(printf '%s' "$remove_cause" | cut -c1-120)
                record deb-lifecycle 0 "evidence: $remove_cause | rc=$rc_remove journal=$([ -e "$JOURNAL" ] || [ -L "$JOURNAL" ] && echo present || echo gone) fragment=$([ -e "$FRAGMENT" ] || [ -L "$FRAGMENT" ] && echo present || echo gone)"
            else
                remove_tail=$([ -f "$remove_evidence" ] && tail -n 12 "$remove_evidence" 2>/dev/null || true)
                record deb-lifecycle 0 "evidence: $remove_tail | rc=$rc_remove journal=$([ -e "$JOURNAL" ] || [ -L "$JOURNAL" ] && echo present || echo gone) fragment=$([ -e "$FRAGMENT" ] || [ -L "$FRAGMENT" ] && echo present || echo gone)"
            fi
        fi
    else
        record deb-lifecycle 0 "state=$state_after owners before=$owners_before after=$owners_after version=$version"
    fi
}

case_post_release_update() {
    apt-get upgrade -y >/dev/null 2>>"$LOG"
    rc=$?
    a=$(version_of nvidia-driver-fixture-a)
    b=$(version_of nvidia-driver-fixture-b)
    c=$(version_of fixture-ctl)
    if [ $rc -eq 0 ] && [ "$a" = 2.0 ] && [ "$b" = 2.0 ] && [ "$c" = 5.0 ]; then
        record post-release-update 1 "after release the protected fixtures upgraded to 2.0"
    else
        record post-release-update 0 "post-release rc=$rc versions a=$a b=$b ctl=$c"
    fi
}

case_final_residue() {
    residue=""
    if [ -f "$STAGE/modinfo.fixture" ]; then
        if restore_modinfo_fixture; then
            :
        else
            residue="$residue modinfo"
        fi
    fi
    if [ -e "$FRAGMENT" ] || [ -L "$FRAGMENT" ]; then residue="$residue fragment"; fi
    if [ -e "$JOURNAL" ] || [ -L "$JOURNAL" ]; then residue="$residue journal"; fi
    if [ -e "$JOURNAL_STAGE" ] || [ -L "$JOURNAL_STAGE" ]; then residue="$residue journal-stage"; fi
    leftovers=$(ls -A /etc/apt/preferences.d/ 2>/dev/null | grep '^90qol-' || true)
    [ -n "$leftovers" ] && residue="$residue pref-leftovers"
    staged_left=$(ls -A /etc/apt/preferences.d/ 2>/dev/null | grep -c 'qol-stage-' || true)
    [ "$staged_left" != 0 ] && residue="$residue staged"
    state=$([ -e "$JOURNAL" ] || [ -L "$JOURNAL" ] && echo active || echo absent)
    if [ "$state" != absent ]; then
        residue="$residue state=$state"
    fi
    provider_left=$(dpkg-query -W -f '${Status}' "$PROVIDER" 2>/dev/null)
    case "$provider_left" in
        *"install ok installed"*) residue="$residue provider-fixture" ;;
        "") ;;
        *) residue="$residue provider-$provider_left" ;;
    esac
    if remove_owned_stage "$STAGE"; then
        :
    else
        residue="$residue stage-cleanup"
    fi
    if [ -z "$residue" ]; then
        record final-residue 1 "no qol-owned fragment, journal, staged, preferences leftovers, modinfo fixture, or guest-only provider remain (lstat semantics); the phase-owned staging directory was removed"
    else
        record final-residue 0 "residue:$residue"
    fi
}

if [ "$PHASE" = contract ]; then
    case_contract_fixture
    case_package_contract
    case_package_contract_active
    case_contract_fail_closed
    case_contract_lifecycle
    case_final_residue
elif [ "$PHASE" = phase1 ]; then
    setup
    case_deb_setup

    case_journal_direct_cycle
    case_journal_operator_neighbor
    case_journal_fixed_stage_occupied
    case_collision
    case_adopt_pins
    case_module_version_snapshotted
    case_status_as_user
    case_upgrade
    case_full_upgrade
    case_unattended
    case_control_advances
    case_drift_and_release
    case_ownership
    case_owner_release_order
    case_owner_idempotence
    case_cross_process_lock
    case_update_continuity
    case_raw_artifact_gate
    install_sandbox_adapter
    case_journal_valid_stage_recovery
    case_boundary_a
    case_boundary_b
    case_boundary_link
    case_boundary_publish
    case_boundary_c
    case_fail_next_publish_fsync
    case_fail_next_release_fsync
    case_exact_copy_staged_collision
    case_journal_wrong_inode_stage_preserved
    case_dangling_fragment_residue
    case_nofollow_dir_swap
    case_interrupted_reboot
else
    export QOL_RESIDENT_SANDBOX_CARRIER=1
    case_reboot_resume
    case_staged_fault_recovery
    restore_production_adapter
    case_deb_lifecycle
    case_post_release_update
    case_final_residue
fi

os_id=$(sed -n 's/^ID=//p' /etc/os-release | tr -d '"')
os_version=$(sed -n 's/^VERSION_ID=//p' /etc/os-release | tr -d '"')
total=$(wc -l < "$RESULTS")
passed=$(grep -c '"pass":true' "$RESULTS" || true)
printf '{"workflow":"%s","completed":true,"os":{"id":"%s","version":"%s"},"cases":[' \
    "$WAVE2_WORKFLOW_ID" "$os_id" "$os_version" > "$RESULTS.tmp"
paste -sd, "$RESULTS" >> "$RESULTS.tmp"
printf '],"summary":{"total":%s,"passed":%s}}\n' "$total" "$passed" >> "$RESULTS.tmp"
mv "$RESULTS.tmp" "$RESULTS"
echo "w2 done: $passed/$total passed"
exit 0
