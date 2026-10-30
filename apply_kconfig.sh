#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CONFIG_FILE="${ROOT}/kernel_config.txt"
KERNEL_DIR="${1:-${ROOT}}"
DOTCONFIG="${KERNEL_DIR}/.config"
CONFIG_TOOL="${KERNEL_DIR}/scripts/config"
BACKUP_FILE="${KERNEL_DIR}/config_nonwatchseal.bak"

if [[ ! -f "$CONFIG_FILE" ]]; then
    echo "Error: kernel_config.txt not found: $CONFIG_FILE" >&2
    exit 1
fi

if [[ ! -f "$DOTCONFIG" ]]; then
    echo "Error: kernel .config not found: $DOTCONFIG" >&2
    echo "Generate the initial kernel configuration first." >&2
    exit 1
fi

if [[ ! -x "$CONFIG_TOOL" ]]; then
    echo "Error: scripts/config not found or not executable." >&2
    exit 1
fi

if [[ ! -f "$BACKUP_FILE" ]]; then
    cp -p "$DOTCONFIG" "$BACKUP_FILE"
    echo "Backup created: $BACKUP_FILE"
else
    echo "Backup already exists; preserving: $BACKUP_FILE"
fi

while IFS= read -r line || [[ -n "$line" ]]; do
    [[ "$line" =~ ^[[:space:]]*# ]] && continue
    [[ "$line" =~ ^[[:space:]]*$ ]] && continue

    if [[ "$line" =~ ^(CONFIG_[A-Za-z0-9_]+)=(y|m|n|\".*\"|[0-9]+)$ ]]; then
        key="${BASH_REMATCH[1]#CONFIG_}"
        value="${BASH_REMATCH[2]}"

        case "$value" in
            y)
                "$CONFIG_TOOL" --file "$DOTCONFIG" --enable "$key"
                ;;
            m)
                "$CONFIG_TOOL" --file "$DOTCONFIG" --module "$key"
                ;;
            n)
                "$CONFIG_TOOL" --file "$DOTCONFIG" --disable "$key"
                ;;
            *)
                "$CONFIG_TOOL" --file "$DOTCONFIG" --set-val "$key" "$value"
                ;;
        esac

        echo "Applied: CONFIG_${key}=${value}"
    else
        echo "Warning: ignoring unsupported configuration line: $line" >&2
    fi
done < "$CONFIG_FILE"

echo "Resolving configuration dependencies..."
make -C "$KERNEL_DIR" olddefconfig

echo "Configuration applied successfully."
echo "Backup: $BACKUP_FILE"
echo "Review the changes with:"
echo "diff -u '$BACKUP_FILE' '$DOTCONFIG'"

