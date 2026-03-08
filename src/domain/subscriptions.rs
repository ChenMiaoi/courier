//! Static subscription templates for the mailbox picker.
//!
//! Keeping this list in code makes startup deterministic and offline-friendly;
//! runtime persistence only stores user choices, not a second mutable catalog.

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SubscriptionCategory {
    LinuxSubsystem,
    QemuSubsystem,
}

impl SubscriptionCategory {
    pub const ALL: [Self; 2] = [Self::LinuxSubsystem, Self::QemuSubsystem];

    pub const fn label(self) -> &'static str {
        match self {
            Self::LinuxSubsystem => "linux subsystem",
            Self::QemuSubsystem => "qemu subsystem",
        }
    }

    pub const fn sort_rank(self) -> u8 {
        match self {
            Self::LinuxSubsystem => 1,
            Self::QemuSubsystem => 2,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SubscriptionTemplate {
    pub mailbox: &'static str,
    #[allow(dead_code)]
    pub description: &'static str,
    pub category: SubscriptionCategory,
}

// Keep a curated snapshot in-tree so default subscriptions do not depend on a
// network fetch or on the source site remaining stable at startup time.
pub const DEFAULT_SUBSCRIPTIONS: &[SubscriptionTemplate] = &[
    SubscriptionTemplate {
        mailbox: "arm-scmi",
        description: "SCMI firmware and drivers",
        category: SubscriptionCategory::LinuxSubsystem,
    },
    SubscriptionTemplate {
        mailbox: "audit",
        description: "Audit system development",
        category: SubscriptionCategory::LinuxSubsystem,
    },
    SubscriptionTemplate {
        mailbox: "autofs",
        description: "AutoFS development",
        category: SubscriptionCategory::LinuxSubsystem,
    },
    SubscriptionTemplate {
        mailbox: "backports",
        description: "Linux backports project",
        category: SubscriptionCategory::LinuxSubsystem,
    },
    SubscriptionTemplate {
        mailbox: "bpf",
        description: "BPF list",
        category: SubscriptionCategory::LinuxSubsystem,
    },
    SubscriptionTemplate {
        mailbox: "ceph-devel",
        description: "CEPH filesystem development",
        category: SubscriptionCategory::LinuxSubsystem,
    },
    SubscriptionTemplate {
        mailbox: "cgroups",
        description: "Linux cgroups development",
        category: SubscriptionCategory::LinuxSubsystem,
    },
    SubscriptionTemplate {
        mailbox: "devicetree",
        description: "Devicetree",
        category: SubscriptionCategory::LinuxSubsystem,
    },
    SubscriptionTemplate {
        mailbox: "dmaengine",
        description: "DMA engine development",
        category: SubscriptionCategory::LinuxSubsystem,
    },
    SubscriptionTemplate {
        mailbox: "io-uring",
        description: "Linux io_uring development",
        category: SubscriptionCategory::LinuxSubsystem,
    },
    SubscriptionTemplate {
        mailbox: "kernel-janitors",
        description: "Kernel cleanups",
        category: SubscriptionCategory::LinuxSubsystem,
    },
    SubscriptionTemplate {
        mailbox: "kvm",
        description: "Kernel virtualization (KVM)",
        category: SubscriptionCategory::LinuxSubsystem,
    },
    SubscriptionTemplate {
        mailbox: "linux-acpi",
        description: "Linux ACPI",
        category: SubscriptionCategory::LinuxSubsystem,
    },
    SubscriptionTemplate {
        mailbox: "linux-api",
        description: "Linux userland API",
        category: SubscriptionCategory::LinuxSubsystem,
    },
    SubscriptionTemplate {
        mailbox: "linux-arch",
        description: "Linux architecture",
        category: SubscriptionCategory::LinuxSubsystem,
    },
    SubscriptionTemplate {
        mailbox: "linux-arm-msm",
        description: "Linux ARM-MSM",
        category: SubscriptionCategory::LinuxSubsystem,
    },
    SubscriptionTemplate {
        mailbox: "linux-block",
        description: "Linux block layer",
        category: SubscriptionCategory::LinuxSubsystem,
    },
    SubscriptionTemplate {
        mailbox: "linux-bluetooth",
        description: "Linux bluetooth",
        category: SubscriptionCategory::LinuxSubsystem,
    },
    SubscriptionTemplate {
        mailbox: "linux-btrfs",
        description: "Linux Btrfs",
        category: SubscriptionCategory::LinuxSubsystem,
    },
    SubscriptionTemplate {
        mailbox: "linux-can",
        description: "Linux CAN",
        category: SubscriptionCategory::LinuxSubsystem,
    },
    SubscriptionTemplate {
        mailbox: "linux-cifs",
        description: "Linux CIFS",
        category: SubscriptionCategory::LinuxSubsystem,
    },
    SubscriptionTemplate {
        mailbox: "linux-clk",
        description: "Linux clock framework",
        category: SubscriptionCategory::LinuxSubsystem,
    },
    SubscriptionTemplate {
        mailbox: "linux-crypto",
        description: "Linux crypto layer",
        category: SubscriptionCategory::LinuxSubsystem,
    },
    SubscriptionTemplate {
        mailbox: "linux-cxl",
        description: "Linux CXL",
        category: SubscriptionCategory::LinuxSubsystem,
    },
    SubscriptionTemplate {
        mailbox: "linux-doc",
        description: "Linux documentation",
        category: SubscriptionCategory::LinuxSubsystem,
    },
    SubscriptionTemplate {
        mailbox: "linux-efi",
        description: "Linux EFI",
        category: SubscriptionCategory::LinuxSubsystem,
    },
    SubscriptionTemplate {
        mailbox: "linux-embedded",
        description: "Embedded Linux",
        category: SubscriptionCategory::LinuxSubsystem,
    },
    SubscriptionTemplate {
        mailbox: "linux-ext4",
        description: "Linux ext4",
        category: SubscriptionCategory::LinuxSubsystem,
    },
    SubscriptionTemplate {
        mailbox: "linux-fsdevel",
        description: "Linux filesystem development",
        category: SubscriptionCategory::LinuxSubsystem,
    },
    SubscriptionTemplate {
        mailbox: "linux-gpio",
        description: "Linux GPIO",
        category: SubscriptionCategory::LinuxSubsystem,
    },
    SubscriptionTemplate {
        mailbox: "linux-hardening",
        description: "Linux hardening",
        category: SubscriptionCategory::LinuxSubsystem,
    },
    SubscriptionTemplate {
        mailbox: "linux-hwmon",
        description: "Linux hardware monitor",
        category: SubscriptionCategory::LinuxSubsystem,
    },
    SubscriptionTemplate {
        mailbox: "linux-i2c",
        description: "Linux I2C",
        category: SubscriptionCategory::LinuxSubsystem,
    },
    SubscriptionTemplate {
        mailbox: "linux-input",
        description: "Linux input/HID",
        category: SubscriptionCategory::LinuxSubsystem,
    },
    SubscriptionTemplate {
        mailbox: "linux-kbuild",
        description: "Linux kbuild/kconfig",
        category: SubscriptionCategory::LinuxSubsystem,
    },
    SubscriptionTemplate {
        mailbox: "linux-kernel",
        description: "Linux kernel mailing list",
        category: SubscriptionCategory::LinuxSubsystem,
    },
    SubscriptionTemplate {
        mailbox: "linux-kselftest",
        description: "Linux selftest",
        category: SubscriptionCategory::LinuxSubsystem,
    },
    SubscriptionTemplate {
        mailbox: "linux-media",
        description: "Linux media controller",
        category: SubscriptionCategory::LinuxSubsystem,
    },
    SubscriptionTemplate {
        mailbox: "linux-mm",
        description: "Linux memory-management discussions",
        category: SubscriptionCategory::LinuxSubsystem,
    },
    SubscriptionTemplate {
        mailbox: "linux-next",
        description: "Linux-next discussions",
        category: SubscriptionCategory::LinuxSubsystem,
    },
    SubscriptionTemplate {
        mailbox: "linux-nfs",
        description: "Linux NFS",
        category: SubscriptionCategory::LinuxSubsystem,
    },
    SubscriptionTemplate {
        mailbox: "linux-openrisc",
        description: "Linux OpenRISC",
        category: SubscriptionCategory::LinuxSubsystem,
    },
    SubscriptionTemplate {
        mailbox: "linux-pci",
        description: "Linux PCI subsystem",
        category: SubscriptionCategory::LinuxSubsystem,
    },
    SubscriptionTemplate {
        mailbox: "linux-pm",
        description: "Linux power management",
        category: SubscriptionCategory::LinuxSubsystem,
    },
    SubscriptionTemplate {
        mailbox: "linux-rdma",
        description: "Linux RDMA / InfiniBand",
        category: SubscriptionCategory::LinuxSubsystem,
    },
    SubscriptionTemplate {
        mailbox: "linux-scsi",
        description: "Linux SCSI subsystem",
        category: SubscriptionCategory::LinuxSubsystem,
    },
    SubscriptionTemplate {
        mailbox: "linux-security-module",
        description: "Linux security modules",
        category: SubscriptionCategory::LinuxSubsystem,
    },
    SubscriptionTemplate {
        mailbox: "linux-serial",
        description: "Linux serial subsystem",
        category: SubscriptionCategory::LinuxSubsystem,
    },
    SubscriptionTemplate {
        mailbox: "linux-sound",
        description: "Linux sound subsystem",
        category: SubscriptionCategory::LinuxSubsystem,
    },
    SubscriptionTemplate {
        mailbox: "linux-spi",
        description: "Linux SPI subsystem",
        category: SubscriptionCategory::LinuxSubsystem,
    },
    SubscriptionTemplate {
        mailbox: "linux-tegra",
        description: "Linux Tegra",
        category: SubscriptionCategory::LinuxSubsystem,
    },
    SubscriptionTemplate {
        mailbox: "linux-trace-devel",
        description: "Linux trace development",
        category: SubscriptionCategory::LinuxSubsystem,
    },
    SubscriptionTemplate {
        mailbox: "linux-usb",
        description: "Linux USB",
        category: SubscriptionCategory::LinuxSubsystem,
    },
    SubscriptionTemplate {
        mailbox: "linux-wireless",
        description: "Linux wireless",
        category: SubscriptionCategory::LinuxSubsystem,
    },
    SubscriptionTemplate {
        mailbox: "mm-commits",
        description: "Linux MM tree commits",
        category: SubscriptionCategory::LinuxSubsystem,
    },
    SubscriptionTemplate {
        mailbox: "netdev",
        description: "Netdev list",
        category: SubscriptionCategory::LinuxSubsystem,
    },
    SubscriptionTemplate {
        mailbox: "netfilter",
        description: "Linux netfilter",
        category: SubscriptionCategory::LinuxSubsystem,
    },
    SubscriptionTemplate {
        mailbox: "rust-for-linux",
        description: "Rust for Linux",
        category: SubscriptionCategory::LinuxSubsystem,
    },
    SubscriptionTemplate {
        mailbox: "selinux",
        description: "SELinux development",
        category: SubscriptionCategory::LinuxSubsystem,
    },
    SubscriptionTemplate {
        mailbox: "stable",
        description: "Linux stable discussions",
        category: SubscriptionCategory::LinuxSubsystem,
    },
    SubscriptionTemplate {
        mailbox: "workflows",
        description: "Maintainer workflows",
        category: SubscriptionCategory::LinuxSubsystem,
    },
    SubscriptionTemplate {
        mailbox: "xdp-newbies",
        description: "XDP newbie discussions",
        category: SubscriptionCategory::LinuxSubsystem,
    },
    SubscriptionTemplate {
        mailbox: "qemu-arm",
        description: "QEMU ARM targets and machines",
        category: SubscriptionCategory::QemuSubsystem,
    },
    SubscriptionTemplate {
        mailbox: "qemu-block",
        description: "QEMU block layer",
        category: SubscriptionCategory::QemuSubsystem,
    },
    SubscriptionTemplate {
        mailbox: "qemu-commits",
        description: "QEMU commit notifications",
        category: SubscriptionCategory::QemuSubsystem,
    },
    SubscriptionTemplate {
        mailbox: "qemu-devel",
        description: "QEMU development",
        category: SubscriptionCategory::QemuSubsystem,
    },
    SubscriptionTemplate {
        mailbox: "qemu-ppc",
        description: "QEMU PowerPC targets",
        category: SubscriptionCategory::QemuSubsystem,
    },
    SubscriptionTemplate {
        mailbox: "qemu-riscv",
        description: "QEMU RISC-V targets",
        category: SubscriptionCategory::QemuSubsystem,
    },
    SubscriptionTemplate {
        mailbox: "qemu-rust",
        description: "Rust in QEMU",
        category: SubscriptionCategory::QemuSubsystem,
    },
    SubscriptionTemplate {
        mailbox: "qemu-s390x",
        description: "QEMU s390x targets",
        category: SubscriptionCategory::QemuSubsystem,
    },
    SubscriptionTemplate {
        mailbox: "qemu-stable",
        description: "QEMU stable backports",
        category: SubscriptionCategory::QemuSubsystem,
    },
    SubscriptionTemplate {
        mailbox: "qemu-trivial",
        description: "QEMU trivial patches",
        category: SubscriptionCategory::QemuSubsystem,
    },
];

pub fn uses_gnu_qemu_archive(mailbox: &str) -> bool {
    let normalized = mailbox.trim();
    !normalized.is_empty() && normalized.to_ascii_lowercase().starts_with("qemu-")
}

pub fn category_for_mailbox(mailbox: &str) -> Option<SubscriptionCategory> {
    let normalized = mailbox.trim();
    if normalized.is_empty() || normalized.eq_ignore_ascii_case("INBOX") {
        return None;
    }

    DEFAULT_SUBSCRIPTIONS
        .iter()
        .find(|entry| entry.mailbox.eq_ignore_ascii_case(normalized))
        .map(|entry| entry.category)
        .or_else(|| {
            // Persisted state can refer to ad-hoc QEMU lists that are not part of
            // the curated snapshot yet; keep them under the QEMU bucket so the
            // subscription pane does not reshuffle after a restart.
            if uses_gnu_qemu_archive(normalized) {
                Some(SubscriptionCategory::QemuSubsystem)
            } else {
                None
            }
        })
}

#[cfg(test)]
mod tests {
    use super::{SubscriptionCategory, category_for_mailbox, uses_gnu_qemu_archive};

    #[test]
    fn qemu_mailboxes_match_case_insensitively() {
        assert!(uses_gnu_qemu_archive("QEMU-devel"));
        assert_eq!(
            category_for_mailbox("QEMU-devel"),
            Some(SubscriptionCategory::QemuSubsystem)
        );
    }
}
