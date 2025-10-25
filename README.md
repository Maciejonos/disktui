<div align="center">
  <h2>TUI for disk management and partitioning</h2>
</div>

## Demo
![disktui-demo](https://github.com/user-attachments/assets/dbb9b1b5-2578-4e0b-b61c-48629c71b8e3)

## 💡 Prerequisites

### A Linux based system.

### Required packages
- `parted` - partition management
- `e2fsprogs` - ext4 filesystem support

### Optional packages
- `dosfstools` - FAT32 filesystem support
- `ntfs-3g` - NTFS filesystem support
- `exfatprogs` - exFAT filesystem support
- `btrfs-progs` - Btrfs filesystem support
- `xfsprogs` - XFS filesystem support
- `smartmontools` - SMART disk health monitoring

> [!WARNING]
> This tool can perform destructive disk operations.

## 🚀 Installation

### 📥 Binary release

You can download the pre-built binaries from the [release page](https://github.com/Maciejonos/disktui/releases)

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

> [!IMPORTANT]
> disktui requires root privileges to perform disk operations.

```bash
sudo disktui
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

`d`: Delete selected partition.

## Theming
disktui follows terminal ANSI colors

## ⚖️ License

MIT
