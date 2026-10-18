//! One private job per OS-sandboxed process. Stdout is binary protocol only.
#[cfg(windows)]
mod dispatch;

fn main() {
    // A panic never prints its document-derived payload or source path. The
    // incomplete reply/nonzero exit remains a refusal at the parent boundary.
    std::panic::set_hook(Box::new(|_| {}));
    #[cfg(windows)]
    let code = run();
    #[cfg(not(windows))]
    let code = 70;
    std::process::exit(code);
}

#[cfg(windows)]
fn run() -> i32 {
    let args: Vec<_> = std::env::args_os().collect();
    if args.len() != 2 || args[1] != "--document-worker-v1" {
        return 70;
    }
    let Ok(process_memory) = vw_document_win::verify_worker_token() else {
        return 70;
    };
    let Ok(executable) = std::env::current_exe() else {
        return 70;
    };
    let Some(directory) = executable.parent() else {
        return 70;
    };
    let mut input = std::io::stdin().lock();
    let Ok(job) = vw_document_protocol::read_job(&mut input) else {
        return 71;
    };
    let mut output = std::io::stdout().lock();
    let Ok(mut reply) = vw_document_protocol::ReplyWriter::new(
        &mut output,
        job.binding,
        job.request.limits.output_bytes,
    ) else {
        return 71;
    };
    if job.request.limits.memory_bytes != process_memory {
        let _ = reply.fault(vw_document_protocol::FaultCode::Memory);
        return 72;
    }
    let binding = job.binding;
    match dispatch::execute(job, &directory.join("pdfium.dll")) {
        Ok(result) => {
            if reply.metadata(&result.metadata).is_err() || reply.data(&result.bytes).is_err() {
                return 73;
            }
            match reply.finish() {
                Ok(done) if done.binding == binding => 0,
                _ => 73,
            }
        }
        Err(code) => {
            let _ = reply.fault(code);
            72
        }
    }
}
