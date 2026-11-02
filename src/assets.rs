//! Every image the app draws, embedded in the executable and uploaded as textures at startup.

use std::collections::HashMap;

use eframe::egui::{self, ColorImage, Context, TextureFilter, TextureHandle, TextureOptions, TextureWrapMode, Vec2, vec2};

macro_rules! asset {
    ($path:literal) => {
        include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/", $path)) as &[u8]
    };
}

pub struct Sprite {
    pub tex: TextureHandle,
    /// Size of the source image in pixels.
    pub size: Vec2,
}

impl Sprite {
    pub fn id(&self) -> egui::TextureId {
        self.tex.id()
    }
}

pub struct Assets {
    pub navy: Sprite,
    pub parchment: Sprite,
    pub grain: Sprite,
    /// Puppet parts, by file name.
    pub detective: HashMap<&'static str, Sprite>,
    pub props: HashMap<&'static str, Sprite>,
}

const DETECTIVE: &[(&str, &[u8])] = &[
    ("torso", asset!("detective/torso.png")),
    ("trousers", asset!("detective/trousers.png")),
    ("leg_left", asset!("detective/leg_left.png")),
    ("leg_right", asset!("detective/leg_right.png")),
    ("boot_left", asset!("detective/boot_left.png")),
    ("boot_right", asset!("detective/boot_right.png")),
    ("coat_left", asset!("detective/coat_left.png")),
    ("coat_right", asset!("detective/coat_right.png")),
    ("sleeve_left", asset!("detective/sleeve_left.png")),
    ("sleeve_right", asset!("detective/sleeve_right.png")),
    ("forearm_left", asset!("detective/forearm_left.png")),
    ("forearm_right", asset!("detective/forearm_right.png")),
    ("head", asset!("detective/head_base.png")),
    ("head_nose", asset!("detective/head_nose.png")),
    ("head_moustache", asset!("detective/head_moustache.png")),
    ("hat", asset!("detective/hat.png")),
    ("glass", asset!("detective/glass.png")),
    ("glass_frame", asset!("detective/glass_frame.png")),
];

const PROPS: &[(&str, &[u8])] = &[
    ("pen", asset!("props/pen.png")),
    ("teacup", asset!("props/teacup.png")),
    ("books", asset!("props/books.png")),
    ("papers", asset!("props/papers.png")),
    ("compass", asset!("props/compass.png")),
    ("fingerprint", asset!("props/fingerprint.png")),
    ("paperclip", asset!("props/paperclip.png")),
    ("pushpin", asset!("props/pushpin.png")),
];

const SPRITE: TextureOptions = TextureOptions {
    magnification: TextureFilter::Linear,
    minification: TextureFilter::Linear,
    wrap_mode: TextureWrapMode::ClampToEdge,
    mipmap_mode: Some(TextureFilter::Linear),
};

const TILED: TextureOptions = TextureOptions {
    magnification: TextureFilter::Linear,
    minification: TextureFilter::Linear,
    wrap_mode: TextureWrapMode::Repeat,
    mipmap_mode: Some(TextureFilter::Linear),
};

impl Assets {
    pub fn load(ctx: &Context) -> Self {
        let load_all = |list: &[(&'static str, &[u8])]| {
            list.iter().map(|(name, bytes)| (*name, load(ctx, name, bytes, SPRITE))).collect()
        };
        Self {
            navy: load(ctx, "navy_paper", asset!("textures/navy_paper.png"), TILED),
            parchment: load(ctx, "parchment", asset!("textures/parchment.png"), TILED),
            grain: load(ctx, "grain", asset!("textures/grain.png"), TILED),
            detective: load_all(DETECTIVE),
            props: load_all(PROPS),
        }
    }

    pub fn part(&self, name: &str) -> &Sprite {
        &self.detective[name]
    }

    pub fn prop(&self, name: &str) -> &Sprite {
        &self.props[name]
    }
}

fn load(ctx: &Context, name: &str, bytes: &[u8], options: TextureOptions) -> Sprite {
    let img = image::load_from_memory(bytes).unwrap_or_else(|e| panic!("bad embedded image {name}: {e}")).to_rgba8();
    let size = [img.width() as usize, img.height() as usize];
    let color = ColorImage::from_rgba_unmultiplied(size, img.as_raw());
    Sprite { tex: ctx.load_texture(name, color, options), size: vec2(size[0] as f32, size[1] as f32) }
}
