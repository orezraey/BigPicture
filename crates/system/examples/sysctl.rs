//! Ferramenta de teste manual do backend de sistema.
//!
//! ```text
//! cargo run -p bigpicture-system --example sysctl -- list
//! cargo run -p bigpicture-system --example sysctl -- try-mode 1920x1080@60 [--display \\.\DISPLAY1] [--seconds 8]
//! cargo run -p bigpicture-system --example sysctl -- launch notepad.exe
//! ```
//!
//! `try-mode` troca o modo, espera e volta sozinho. A troca é temporária: mesmo se o processo
//! for morto no meio, o Windows restaura o modo salvo quando ele termina.

use std::path::PathBuf;
use std::thread::sleep;
use std::time::Duration;

use bigpicture_system::{DisplayId, DisplayMode, LaunchRequest, SystemControl, WindowId, platform};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let sys = platform();
    println!("backend: {}", sys.backend_name());
    let result = match args.first().map(String::as_str) {
        Some("list") => list(sys.as_ref()),
        Some("try-mode") => try_mode(sys.as_ref(), &args[1..]),
        Some("launch") => launch(sys.as_ref(), &args[1..]),
        Some("open-find-close") => args.get(1).ok_or_else(|| "falta o executável".to_string()).and_then(|exe| {
            let exe = std::path::Path::new(exe);
            let app = sys.launch_app(&LaunchRequest { executable: exe.into(), ..Default::default() }).map_err(|e| e.to_string())?;
            sleep(Duration::from_secs(3));
            let by_pid = sys.windows_of_process(app.pid).map_err(|e| e.to_string())?;
            let by_exe = sys.windows_of_executable(exe).map_err(|e| e.to_string())?;
            println!("pid {}: {} janela(s) pelo pid, {} pelo executável", app.pid, by_pid.len(), by_exe.len());
            for w in by_exe { sys.close_window(w).map_err(|e| e.to_string())?; }
            Ok(())
        }),
        Some("windows-of") => args.get(1).ok_or_else(|| "falta o executável".to_string()).and_then(|exe| {
            let found = sys.windows_of_executable(std::path::Path::new(exe)).map_err(|e| e.to_string())?;
            println!("{} janela(s): {found:?}", found.len());
            Ok(())
        }),
        _ => Err("uso: list | try-mode WxH@HZ [--display ID] [--seconds N] | launch EXE [ARGS...]".into()),
    };
    if let Err(err) = result {
        eprintln!("erro: {err}");
        std::process::exit(1);
    }
}

fn list(sys: &dyn SystemControl) -> Result<(), String> {
    for d in sys.list_displays().map_err(|e| e.to_string())? {
        println!(
            "{}{} ({}) em {:?}: atual {}",
            d.id,
            if d.primary { " [principal]" } else { "" },
            d.name,
            d.position,
            d.current
        );
        let modes: Vec<String> = d.modes.iter().map(ToString::to_string).collect();
        println!("  modos: {}", modes.join(", "));
    }
    Ok(())
}

fn try_mode(sys: &dyn SystemControl, args: &[String]) -> Result<(), String> {
    let mode: DisplayMode = args.first().ok_or("falta o modo")?.parse()?;
    let mut display = None;
    let mut seconds = 8u64;
    let mut it = args[1..].iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--display" => display = it.next().cloned(),
            "--seconds" => seconds = it.next().ok_or("falta N")?.parse().map_err(|e| format!("{e}"))?,
            other => return Err(format!("argumento desconhecido: {other}")),
        }
    }
    let displays = sys.list_displays().map_err(|e| e.to_string())?;
    let target = match display {
        Some(id) => displays.into_iter().find(|d| d.id.0 == id).ok_or(format!("monitor {id} não existe"))?,
        None => displays.into_iter().find(|d| d.primary).ok_or("sem monitor principal")?,
    };
    println!("{}: {} -> {} por {seconds}s", target.id, target.current, mode);
    sys.set_display_mode(&target.id, mode).map_err(|e| e.to_string())?;
    report(sys, &target.id, "durante");
    sleep(Duration::from_secs(seconds));
    sys.restore_display_mode(&target.id).map_err(|e| e.to_string())?;
    report(sys, &target.id, "depois");
    Ok(())
}

fn report(sys: &dyn SystemControl, id: &DisplayId, when: &str) {
    if let Some(d) = sys.list_displays().ok().and_then(|ds| ds.into_iter().find(|d| &d.id == id)) {
        println!("  {when}: {}", d.current);
    }
}

fn launch(sys: &dyn SystemControl, args: &[String]) -> Result<(), String> {
    let exe = args.first().ok_or("falta o executável")?;
    let request = LaunchRequest {
        executable: PathBuf::from(exe),
        args: args[1..].to_vec(),
        working_dir: None,
    };
    // A fica atrás de B, e este processo (sem janela) não está em primeiro plano: é o caso do
    // botão Guide com um jogo na frente do launcher.
    let (pid_a, a) = open(sys, &request)?;
    let (pid_b, b) = open(sys, &request)?;
    println!("A = pid {pid_a} {a:?}, B = pid {pid_b} {b:?}");
    sleep(Duration::from_secs(1));
    println!("minimizando B");
    sys.minimize_window(b).map_err(|e| e.to_string())?;
    sleep(Duration::from_secs(1));
    println!("focando B de novo (restaura)");
    sys.focus_window(b).map_err(|e| e.to_string())?;
    sleep(Duration::from_secs(1));
    println!("trazendo A para a frente");
    let result = sys.bring_launcher_to_front(a).map_err(|e| e.to_string());
    println!("  resultado: {result:?}");
    sleep(Duration::from_secs(1));
    println!("fechando A e B (WM_CLOSE)");
    for window in [a, b] {
        sys.close_window(window).map_err(|e| e.to_string())?;
    }
    sleep(Duration::from_secs(1));
    for pid in [pid_a, pid_b] {
        let open = !sys.windows_of_process(pid).map_err(|e| e.to_string())?.is_empty();
        println!("  pid {pid} ainda com janela: {open}");
    }
    result
}

fn open(sys: &dyn SystemControl, request: &LaunchRequest) -> Result<(u32, WindowId), String> {
    let app = sys.launch_app(request).map_err(|e| e.to_string())?;
    for _ in 0..50 {
        if let Some(&w) = sys.windows_of_process(app.pid).map_err(|e| e.to_string())?.first() {
            return Ok((app.pid, w));
        }
        sleep(Duration::from_millis(100));
    }
    Err("a janela do processo não apareceu (apps que reabrem em outro processo não são achados pelo pid)".into())
}