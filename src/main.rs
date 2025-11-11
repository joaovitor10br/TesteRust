use std::path::PathBuf;
use tempfile::tempdir;
use std::process::Command;
use std::fs;
use std::io;

fn main() -> io::Result<()> {
    // === Caminhos de exemplo (mude conforme necessário) ===
    let iso_path = PathBuf::from("/home/joao/Downloads/linuxmint-22.2-cinnamon-64bit.iso");
    let output_iso = PathBuf::from("/home/joao/mint-firefox.iso");
    let inject_file = PathBuf::from("/home/joao/Downloads/firefox-144.0.2.tar.xz");
    let inject_dest = PathBuf::from("/opt/firefox");

    // === Cria diretório temporário ===
    let tmpdir = tempdir()?;
    let tmp_path = tmpdir.path().to_path_buf();

    let mount_dir = tmp_path.join("iso_mount");
    let work_dir = tmp_path.join("work");

    fs::create_dir_all(&mount_dir)?;
    fs::create_dir_all(&work_dir)?;

    println!("> mount -o loop,ro {} {}", iso_path.display(), mount_dir.display());

    // === Monta a ISO ===
    let status = Command::new("mount")
        .args(["-o", "loop,ro", iso_path.to_str().unwrap(), mount_dir.to_str().unwrap()])
        .status()?;

    if !status.success() {
        eprintln!("❌ Falha ao montar a ISO (tente rodar como root)");
        std::process::exit(1);
    }

    // === Copia conteúdo da ISO ===
    println!("> rsync -aH {} {}", mount_dir.display(), work_dir.display());
    let status = Command::new("rsync")
        .args(["-aH", &format!("{}/", mount_dir.display()), work_dir.to_str().unwrap()])
        .status()?;

    if !status.success() {
        eprintln!("❌ Falha ao copiar conteúdo da ISO");
        std::process::exit(1);
    }

    // === Procura boot loaders ===
    let mut isolinux_path = None;
    let mut grub_efi_path = None;

    for entry in walkdir::WalkDir::new(&work_dir) {
        let entry = entry?;
        let path = entry.path();

        if path.ends_with("isolinux.bin") {
            isolinux_path = Some(path.to_path_buf());
        } else if path.ends_with("efi.img") {
            grub_efi_path = Some(path.to_path_buf());
        }
    }

    if isolinux_path.is_none() && grub_efi_path.is_none() {
        eprintln!("❌ Nenhum arquivo de boot encontrado (isolinux ou grub)");
        let _ = Command::new("umount").arg(&mount_dir).status();
        return Ok(());
    }

    println!(
        "[iso_injector] Detectado boot loader: isolinux: {:?}, grub EFI: {:?}",
        isolinux_path, grub_efi_path
    );

    // === Injeta o arquivo ===
    let dest_path = work_dir.join(inject_dest.strip_prefix("/").unwrap());
    fs::create_dir_all(&dest_path)?;
    fs::copy(&inject_file, dest_path.join(inject_file.file_name().unwrap()))?;
    println!("> Arquivo injetado em {}", dest_path.display());

    // === Gera nova ISO ===
    println!("> xorriso -as mkisofs -o {} -J -r {}", output_iso.display(), work_dir.display());

    let status = Command::new("xorriso")
        .args([
            "-as", "mkisofs",
            "-o", output_iso.to_str().unwrap(),
            "-J",
            "-r",
            "-V", "LINUX_MINT_CUSTOM",
            "-isohybrid-mbr", "/usr/lib/syslinux/bios/isohdpfx.bin",
            "-c", "boot.cat",
            "-b", "isolinux/isolinux.bin",
            "-no-emul-boot",
            "-boot-load-size", "4",
            "-boot-info-table",
            "-eltorito-alt-boot",
            "-e", "boot/grub/efi.img",
            "-no-emul-boot",
            work_dir.to_str().unwrap(),
        ])
        .status()?;

    if !status.success() {
        eprintln!("❌ Falha ao gerar nova ISO");
        std::process::exit(1);
    }

    // === Desmonta ISO ===
    println!("> umount {}", mount_dir.display());
    let _ = Command::new("umount").arg(&mount_dir).status();

    println!("✅ ISO modificada criada com sucesso!");
    println!("📦 Arquivo final: {}", output_iso.display());
    Ok(())
}
