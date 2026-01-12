<div align="center">
  <h2>TUI for disk management and partitioning</h2>
</div>

## Demo
https://github.com/user-attachments/assets/841aec6a-0d4b-4738-a637-a3a27469348e

## 💡 Prerequisites

### A Linux based system.

### Required packages
- `parted` - partition management
- `sfdisk` - partition resizing (usually included with `util-linux`)
- `e2fsprogs` - ext4 filesystem support
- `cryptsetup` - LUKS encryption support

### Optional packages
- `dosfstools` - FAT32 filesystem support
- `ntfs-3g` - NTFS filesystem support
- `exfatprogs` - exFAT filesystem support
- `btrfs-progs` - Btrfs filesystem support
- `xfsprogs` - XFS filesystem support
- `smartmontools` - SMART disk health monitoring

> [!WARNING]
> This tool can perform destructive disk operations. You will be prompted to authenticate for operations requiring sudo.

## 🚀 Installation

### 📦 crates.io

You can install `disktui` from [crates.io](https://crates.io/crates/disktui)

```shell
cargo install disktui
```

### 🐧 Arch Linux

You can install `disktui` from the [AUR](https://aur.archlinux.org/packages/disktui) using an AUR helper like [paru](https://github.com/Morganamilo/paru).

```bash
paru -S disktui
```

### ⚒️ Build from source

Run the following command:

```shell
git clone https://github.com/Maciejonos/disktui
cd disktui
cargo build --release
```

run with
```shell
sudo ./target/release/disktui
```

## 🪄 Usage

```bash
disktui
```

## ⌨️ Keybindings

### Global

`Tab` or `Shift + Tab`: Switch between disks and partitions sections.

`j` or `Down`: Scroll down.

`k` or `Up`: Scroll up.

`?`: Show help.

`q` or `Esc`: Quit the app.

`Ctrl + c`: Force quit.

### Disks

`i`: Show detailed disk information and SMART data.

`f`: Format entire disk with a filesystem.

`p`: Create a new partition table (GPT/MBR).

`n`: Create a new partition.

### Partitions

`f`: Format selected partition.

`m`: Mount/unmount selected partition.

`r`: Resize selected partition (must be unmounted, encrypted partitions cannot be resized).

`d`: Delete selected partition.

`e`: Encrypt partition with LUKS2 (destroys all data).

`l`: Lock/unlock encrypted partition (requires passphrase).

## Theming
disktui follows terminal ANSI colors

## 🔐 LUKS Encryption

Press `e` to encrypt a partition with LUKS2, then `l` to lock/unlock it (requires passphrase). Encrypted partitions show 🔒 (locked) or 🔓 (unlocked) and must be unlocked before mounting or formatting.

## ⚖️ License

MIT
