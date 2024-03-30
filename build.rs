use {
    anyhow::{anyhow, bail, Context},
    shaderc::{CompileOptions, ResolvedInclude},
    std::{
        env,
        fs::{File, OpenOptions},
        io::{self, BufWriter, Write},
        path::{Path, PathBuf},
    },
};

const ROOT: &str = "src/shaders";

fn main() -> anyhow::Result<()> {
    wl_client_builder::Builder::default().build()?;

    println!("cargo:rerun-if-changed={}", ROOT);
    compile_simple("fill.frag")?;
    compile_simple("fill.vert")?;
    Ok(())
}

fn compile_simple(name: &str) -> anyhow::Result<()> {
    let out = format!("{name}.spv").replace("/", "_");
    compile_shader(name, &out).with_context(|| name.to_string())
}

fn compile_shader(name: &str, out: &str) -> anyhow::Result<()> {
    let root = Path::new(ROOT).join(Path::new(name).parent().unwrap());
    let read = |path: &str| std::fs::read_to_string(root.join(path));
    let mut options = CompileOptions::new().unwrap();
    options.set_include_callback(|name, _, _, _| {
        Ok(ResolvedInclude {
            resolved_name: name.to_string(),
            content: read(name).map_err(|e| anyhow!(e).to_string())?,
        })
    });
    let stage = match Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
    {
        "frag" => shaderc::ShaderKind::Fragment,
        "vert" => shaderc::ShaderKind::Vertex,
        n => bail!("Unknown shader stage {}", n),
    };
    let src = std::fs::read_to_string(format!("{}/{}", ROOT, name))?;
    let compiler = shaderc::Compiler::new()?;
    let binary = compiler.compile_into_spirv(&src, stage, name, "main", Some(&options))?;
    let mut file = open(out)?;
    file.write_all(binary.as_binary_u8())?;
    file.flush()?;
    Ok(())
}

fn open(s: &str) -> io::Result<BufWriter<File>> {
    let mut path = PathBuf::from(env::var("OUT_DIR").unwrap());
    path.push(s);
    Ok(BufWriter::new(
        OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(path)?,
    ))
}
