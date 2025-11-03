use clap::Parser;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::io::{self};
use std::env;
use std::ffi::OsStr;
use std::time::Duration;
use std::thread::sleep;
use scopeguard::guard;

/// Macro simples para logs padronizados
macro_rules! log {
    ($($arg:tt)*) => {
        eprintln!("[iso_injector] {}", format!($($arg)*));
    }
}

/// Executa um comando no sistema, exibindo saída em tempo real e falhando em erro.
fn run_cmd<S: AsRef<OsStr> + std::fmt::Debug>(cmd: &str, args: &[S]) -> io::Result<()> {
    eprintln!("> {} {}", cmd, args.iter().map(|a| a.as_ref().to_string_lossy()).collect::<Vec<_>>().join(" "));
    let mut c = Command::new(cmd);
    c.args(args);
    c.stdin(Stdio::inherit());
    c.stdout(Stdio::inherit());
    c.stderr(Stdio::inherit());
    let status = c.status()?;
    if !status.success() {
        return Err(io::Error::new(io::ErrorKind::Other, format!("Command failed: {} {:?}", cmd, args)));
    }
    Ok(())
}

#[derive(Parser, Debug)]
#[command(author, version, about = "Desmonta ISO, injeta pacotes via script e remonta (caso squashfs).")]
struct Args {
    /// Caminho para a ISO de entrada
    #[arg(long)]
    iso: PathBuf,

    /// Caminho para a ISO de saída
    #[arg(long)]
    out: PathBuf,

    /// Script executado dentro do chroot para instalar pacotes (opcional)
    #[arg(long)]
    install_script: Option<PathBuf>,

    /// Use xorriso em vez de genisoimage (padrão: true)
    #[arg(long, default_value_t = true)]
    use_xorriso: bool,
}

/// Verifica se está sendo executado como root
fn is_root() -> bool {
    match Command::new("id").arg("-u").output() {
        Ok(o) => String::from_utf8_lossy(&o.stdout).trim() == "0",
        Err(_) => false,
    }
}

fn main() -> io::Result<()> {
    let a = Args::parse();

    if !is_root() {
        eprintln!("❌ Este programa precisa ser executado como root. Abortando.");
        std::process::exit(1);
    }

    let iso = a.iso.canonicalize()?;
    if !iso.exists() {
        eprintln!("❌ ISO de entrada não existe: {:?}", iso);
        std::process::exit(1);
    }

    // Diretórios temporários
    let base = env::temp_dir().join(format!("iso_injector_{}", chrono::Utc::now().timestamp()));
    let mount_iso = base.join("iso_mount");
    let work = base.join("work");
    let squash_root = base.join("squashfs-root");
    fs::create_dir_all(&mount_iso)?;
    fs::create_dir_all(&work)?;
    fs::create_dir_all(&squash_root)?;

    // Guard de limpeza automática (executa no final ou em erro)
    let _cleanup = guard(base.clone(), |b| {
        let _ = Command::new("umount").arg(mount_iso.as_os_str()).status();
        let _ = fs::remove_dir_all(&b);
    });

    // Montar a ISO
    run_cmd("mount", &[
        OsStr::new("-o"), OsStr::new("loop,ro"),
        iso.as_os_str(),
        mount_iso.as_os_str()
    ])?;

    // Copiar conteúdo
    run_cmd("rsync", &[OsStr::new("-aH"), mount_iso.as_os_str(), work.as_os_str()])?;

    // Verificar squashfs
    let possible_squash = work.join("casper").join("filesystem.squashfs");
    if possible_squash.exists() {
        log!("Encontrado squashfs em {:?}", possible_squash);

        // Extrair squashfs
        run_cmd("unsquashfs", &[OsStr::new("-d"), squash_root.as_os_str(), possible_squash.as_os_str()])?;

        // Bind mounts e DNS
        run_cmd("mount", &[OsStr::new("--bind"), OsStr::new("/dev"), squash_root.join("dev").as_os_str()])?;
        run_cmd("mount", &[OsStr::new("--bind"), OsStr::new("/proc"), squash_root.join("proc").as_os_str()])?;
        run_cmd("mount", &[OsStr::new("--bind"), OsStr::new("/sys"), squash_root.join("sys").as_os_str()])?;
        fs::create_dir_all(squash_root.join("etc"))?;
        run_cmd("bash", &[OsStr::new("-c"), OsStr::new(&format!("cp -L /etc/resolv.conf {}/etc/resolv.conf", squash_root.display()))])?;

        // Executar script dentro do chroot
        if let Some(script) = &a.install_script {
            let target = squash_root.join("tmp").join("install_in_chroot.sh");
            fs::create_dir_all(target.parent().unwrap())?;
            fs::copy(script, &target)?;
            run_cmd("chmod", &[OsStr::new("+x"), target.as_os_str()])?;
            log!("Executando script dentro do chroot...");
            run_cmd("chroot", &[squash_root.as_os_str(), OsStr::new("/bin/bash"), OsStr::new("-c"), OsStr::new("/tmp/install_in_chroot.sh")])?;
        } else {
            log!("Nenhum script de instalação fornecido — você pode abrir o chroot manualmente se quiser.");
            eprintln!("chroot {} /bin/bash", squash_root.display());
        }

        // Sincronizar e reconstruir
        run_cmd::<&str>("sync", &[] as &[&str])?;
        fs::remove_file(&possible_squash).ok();
        log!("Recriando squashfs...");
        run_cmd("mksquashfs", &[squash_root.as_os_str(), possible_squash.as_os_str(), OsStr::new("-noappend")])?;

        // Desmontar binds
        run_cmd("umount", &[squash_root.join("dev").as_os_str()])?;
        run_cmd("umount", &[squash_root.join("proc").as_os_str()])?;
        run_cmd("umount", &[squash_root.join("sys").as_os_str()])?;
    } else {
        log!("Nenhum squashfs detectado, tentando injeção direta.");
        if let Some(script) = &a.install_script {
            let maybe_root = work.join("rootfs");
            if maybe_root.exists() {
                run_cmd("mount", &[OsStr::new("--bind"), OsStr::new("/dev"), maybe_root.join("dev").as_os_str()])?;
                run_cmd("mount", &[OsStr::new("--bind"), OsStr::new("/proc"), maybe_root.join("proc").as_os_str()])?;
                run_cmd("mount", &[OsStr::new("--bind"), OsStr::new("/sys"), maybe_root.join("sys").as_os_str()])?;
                let target = maybe_root.join("tmp").join("install_in_chroot.sh");
                fs::create_dir_all(target.parent().unwrap())?;
                fs::copy(script, &target)?;
                run_cmd("chmod", &[OsStr::new("+x"), target.as_os_str()])?;
                run_cmd("chroot", &[maybe_root.as_os_str(), OsStr::new("/bin/bash"), OsStr::new("-c"), OsStr::new("/tmp/install_in_chroot.sh")])?;
                run_cmd("umount", &[maybe_root.join("dev").as_os_str()])?;
                run_cmd("umount", &[maybe_root.join("proc").as_os_str()])?;
                run_cmd("umount", &[maybe_root.join("sys").as_os_str()])?;
            } else {
                log!("Não encontrei um rootfs, inspecione manualmente '{}'", work.display());
            }
        } else {
            log!("Nada a injetar automaticamente.");
        }
    }

    // Atualizar md5sum.txt se existir
    let md5sum_file = work.join("md5sum.txt");
    if md5sum_file.exists() {
        log!("Atualizando md5sum.txt...");
        run_cmd("bash", &[
            OsStr::new("-c"),
            OsStr::new(&format!(
                "cd {} && find . -type f ! -path './md5sum.txt' -print0 | xargs -0 md5sum > md5sum.txt",
                work.display()
            )),
        ])?;
    }

    // Criar nova ISO
    log!("Criando nova ISO em {:?}", a.out);
    if a.use_xorriso {
        run_cmd("xorriso", &[
            OsStr::new("-as"), OsStr::new("mkisofs"),
            OsStr::new("-o"), a.out.as_os_str(),
            OsStr::new("-J"), OsStr::new("-r"),
            work.as_os_str()
        ])?;
    } else {
        run_cmd("genisoimage", &[
            OsStr::new("-o"), a.out.as_os_str(),
            OsStr::new("-J"), OsStr::new("-r"),
            work.as_os_str()
        ])?;
    }

    // Desmontar e limpar
    run_cmd("umount", &[mount_iso.as_os_str()])?;
    sleep(Duration::from_millis(200));
    fs::remove_dir_all(&base).ok();

    log!("Feito! Nova ISO: {:?}", a.out);
    Ok(())
}
