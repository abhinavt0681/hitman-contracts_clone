//! Lists a TEX archive and writes every texture as raw RGBA (`id_name_WxH.rgba`).
use hitman_clone::glacier::tex::TexArchive;
fn main() -> anyhow::Result<()> {
    let a: Vec<String> = std::env::args().collect();
    let tex = TexArchive::parse(std::fs::read(&a[1])?)?;
    for id in tex.ids().collect::<Vec<_>>() {
        if let Some(t) = tex.decode(id)? {
            println!("{id} {} {:?} {}x{}", t.name, t.format, t.width, t.height);
            if let Some(dir) = a.get(2) {
                std::fs::write(format!("{dir}/{id}_{}_{}x{}.rgba", t.name.replace('/', "_"), t.width, t.height), &t.rgba)?;
            }
        }
    }
    Ok(())
}
