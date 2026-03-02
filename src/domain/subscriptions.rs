#[derive(Debug, Clone, Copy)]
pub struct SubscriptionTemplate {
    pub mailbox: &'static str,
    pub description: &'static str,
}

// Snapshot from https://subspace.kernel.org/vger.kernel.org.html (M2 baseline).
pub const VGER_SUBSCRIPTIONS: &[SubscriptionTemplate] = &[
    SubscriptionTemplate {
        mailbox: "arm-scmi",
        description: "SCMI firmware and drivers",
    },
    SubscriptionTemplate {
        mailbox: "audit",
        description: "Audit system development",
    },
    SubscriptionTemplate {
        mailbox: "autofs",
        description: "AutoFS development",
    },
    SubscriptionTemplate {
        mailbox: "backports",
        description: "Linux backports project",
    },
    SubscriptionTemplate {
        mailbox: "bpf",
        description: "BPF list",
    },
    SubscriptionTemplate {
        mailbox: "ceph-devel",
        description: "CEPH filesystem development",
    },
    SubscriptionTemplate {
        mailbox: "cgroups",
        description: "Linux cgroups development",
    },
    SubscriptionTemplate {
        mailbox: "devicetree",
        description: "Devicetree",
    },
    SubscriptionTemplate {
        mailbox: "dmaengine",
        description: "DMA engine development",
    },
    SubscriptionTemplate {
        mailbox: "io-uring",
        description: "Linux io_uring development",
    },
    SubscriptionTemplate {
        mailbox: "kernel-janitors",
        description: "Kernel cleanups",
    },
    SubscriptionTemplate {
        mailbox: "kvm",
        description: "Kernel virtualization (KVM)",
    },
    SubscriptionTemplate {
        mailbox: "linux-acpi",
        description: "Linux ACPI",
    },
    SubscriptionTemplate {
        mailbox: "linux-api",
        description: "Linux userland API",
    },
    SubscriptionTemplate {
        mailbox: "linux-arch",
        description: "Linux architecture",
    },
    SubscriptionTemplate {
        mailbox: "linux-arm-msm",
        description: "Linux ARM-MSM",
    },
    SubscriptionTemplate {
        mailbox: "linux-block",
        description: "Linux block layer",
    },
    SubscriptionTemplate {
        mailbox: "linux-bluetooth",
        description: "Linux bluetooth",
    },
    SubscriptionTemplate {
        mailbox: "linux-btrfs",
        description: "Linux Btrfs",
    },
    SubscriptionTemplate {
        mailbox: "linux-can",
        description: "Linux CAN",
    },
    SubscriptionTemplate {
        mailbox: "linux-cifs",
        description: "Linux CIFS",
    },
    SubscriptionTemplate {
        mailbox: "linux-clk",
        description: "Linux clock framework",
    },
    SubscriptionTemplate {
        mailbox: "linux-crypto",
        description: "Linux crypto layer",
    },
    SubscriptionTemplate {
        mailbox: "linux-cxl",
        description: "Linux CXL",
    },
    SubscriptionTemplate {
        mailbox: "linux-doc",
        description: "Linux documentation",
    },
    SubscriptionTemplate {
        mailbox: "linux-efi",
        description: "Linux EFI",
    },
    SubscriptionTemplate {
        mailbox: "linux-embedded",
        description: "Embedded Linux",
    },
    SubscriptionTemplate {
        mailbox: "linux-ext4",
        description: "Linux ext4",
    },
    SubscriptionTemplate {
        mailbox: "linux-fsdevel",
        description: "Linux filesystem development",
    },
    SubscriptionTemplate {
        mailbox: "linux-gpio",
        description: "Linux GPIO",
    },
    SubscriptionTemplate {
        mailbox: "linux-hardening",
        description: "Linux hardening",
    },
    SubscriptionTemplate {
        mailbox: "linux-hwmon",
        description: "Linux hardware monitor",
    },
    SubscriptionTemplate {
        mailbox: "linux-i2c",
        description: "Linux I2C",
    },
    SubscriptionTemplate {
        mailbox: "linux-input",
        description: "Linux input/HID",
    },
    SubscriptionTemplate {
        mailbox: "linux-kbuild",
        description: "Linux kbuild/kconfig",
    },
    SubscriptionTemplate {
        mailbox: "linux-kernel",
        description: "Linux kernel mailing list",
    },
    SubscriptionTemplate {
        mailbox: "linux-kselftest",
        description: "Linux selftest",
    },
    SubscriptionTemplate {
        mailbox: "linux-media",
        description: "Linux media controller",
    },
    SubscriptionTemplate {
        mailbox: "linux-mm",
        description: "Linux memory-management discussions",
    },
    SubscriptionTemplate {
        mailbox: "linux-next",
        description: "Linux-next discussions",
    },
    SubscriptionTemplate {
        mailbox: "linux-nfs",
        description: "Linux NFS",
    },
    SubscriptionTemplate {
        mailbox: "linux-openrisc",
        description: "Linux OpenRISC",
    },
    SubscriptionTemplate {
        mailbox: "linux-pci",
        description: "Linux PCI subsystem",
    },
    SubscriptionTemplate {
        mailbox: "linux-pm",
        description: "Linux power management",
    },
    SubscriptionTemplate {
        mailbox: "linux-rdma",
        description: "Linux RDMA / InfiniBand",
    },
    SubscriptionTemplate {
        mailbox: "linux-scsi",
        description: "Linux SCSI subsystem",
    },
    SubscriptionTemplate {
        mailbox: "linux-security-module",
        description: "Linux security modules",
    },
    SubscriptionTemplate {
        mailbox: "linux-serial",
        description: "Linux serial subsystem",
    },
    SubscriptionTemplate {
        mailbox: "linux-sound",
        description: "Linux sound subsystem",
    },
    SubscriptionTemplate {
        mailbox: "linux-spi",
        description: "Linux SPI subsystem",
    },
    SubscriptionTemplate {
        mailbox: "linux-tegra",
        description: "Linux Tegra",
    },
    SubscriptionTemplate {
        mailbox: "linux-trace-devel",
        description: "Linux trace development",
    },
    SubscriptionTemplate {
        mailbox: "linux-usb",
        description: "Linux USB",
    },
    SubscriptionTemplate {
        mailbox: "linux-wireless",
        description: "Linux wireless",
    },
    SubscriptionTemplate {
        mailbox: "mm-commits",
        description: "Linux MM tree commits",
    },
    SubscriptionTemplate {
        mailbox: "netdev",
        description: "Netdev list",
    },
    SubscriptionTemplate {
        mailbox: "netfilter",
        description: "Linux netfilter",
    },
    SubscriptionTemplate {
        mailbox: "rust-for-linux",
        description: "Rust for Linux",
    },
    SubscriptionTemplate {
        mailbox: "selinux",
        description: "SELinux development",
    },
    SubscriptionTemplate {
        mailbox: "stable",
        description: "Linux stable discussions",
    },
    SubscriptionTemplate {
        mailbox: "workflows",
        description: "Maintainer workflows",
    },
    SubscriptionTemplate {
        mailbox: "xdp-newbies",
        description: "XDP newbie discussions",
    },
];
