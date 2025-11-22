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

    // === Monta ISO ===
    let status = Command::new("mount")
        .args(["-o", "loop,ro", iso_path.to_str().unwrap(), mount_dir.to_str().unwrap()])
        .status()?;
    if !status.success() {
        eprintln!("❌ Falha ao montar ISO");
        std::process::exit(1);
    }

    // === Copia conteúdo da ISO ===
    println!("> rsync -aH {} {}", mount_dir.display(), work_dir.display());
    let status = Command::new("rsync")
        .args(["-aH", &format!("{}/", mount_dir.display()), work_dir.to_str().unwrap()])
        .status()?;
    if !status.success() {
        eprintln!("❌ Erro ao copiar conteúdo da ISO");
        std::process::exit(1);
    }

    // === Procura arquivos de boot EFI modernamente ===
    let mut efi_boot: Option<PathBuf> = None;

    for entry in walkdir::WalkDir::new(&work_dir) {
        let entry = entry?;
        let path = entry.path();

        // Preferência 1 — bootx64.efi
        if path.file_name().map(|f| f == "bootx64.efi").unwrap_or(false) && path.is_file() {
            efi_boot = Some(path.strip_prefix(&work_dir).unwrap().to_path_buf());
            break;
        }
    }

    // Procurar qualquer .efi se não achou bootx64.efi
    if efi_boot.is_none() {
        for entry in walkdir::WalkDir::new(&work_dir) {
            let entry = entry?;
            let path = entry.path();
            if path.extension().map(|e| e == "efi").unwrap_or(false) && path.is_file() {
                efi_boot = Some(path.strip_prefix(&work_dir).unwrap().to_path_buf());
                break;
            }
        }
    }

    println!("[iso_injector] UEFI detectado: {:?}", efi_boot);

    // === Injeta arquivo ===
    let relative_dest = inject_dest.strip_prefix("/").unwrap();
    let dest_dir = work_dir.join(relative_dest);
    fs::create_dir_all(&dest_dir)?;
    let final_file_path = dest_dir.join(inject_file.file_name().unwrap());
    fs::copy(&inject_file, &final_file_path)?;
    println!("> Arquivo injetado em {}", final_file_path.display());

    // === Monta argumentos do xorriso ===
    let mut args = vec![
        "-as", "mkisofs",
        "-o", output_iso.to_str().unwrap(),
        "-J",
        "-r",
        "-V", "LINUX_MINT_CUSTOM",
        "-c", "boot.cat",
        "-b", "isolinux/isolinux.bin",
        "-no-emul-boot",
        "-boot-load-size", "4",
        "-boot-info-table",
    ];

    println!("> xorriso {:?}", args);

    args.push(work_dir.to_str().unwrap());

    let status = Command::new("xorriso")
        .args(&args)
        .status()?;
    if !status.success() {
        eprintln!("❌ Erro ao gerar nova ISO");
        std::process::exit(1);
    }

    // === Desmonta ISO ===
    let _ = Command::new("umount").arg(&mount_dir).status();
    println!("> umount {}", mount_dir.display());

    println!("✅ ISO modificada criada com sucesso!");
    println!("📦 Arquivo final: {}", output_iso.display());
    Ok(())
}
