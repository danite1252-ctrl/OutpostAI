//! Workflow packs: a ComfyUI graph (API format) plus a description of its inputs
//! and the model files it needs. Every Create lane is one pack, so the desktop app,
//! the phone and other apps all drive it with the same form or API call.
//!
//! A pack is two files under `packs/<id>/`: `pack.json` (this module's [`Pack`]) and
//! `graph.json` (the graph exported from ComfyUI in API format). Built-in packs are
//! compiled into Outpost.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use utoipa::ToSchema;

use crate::catalog::Fit;

/// What a pack makes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Lane {
    TextToImage,
    EditImage,
    TextToVideo,
    ImageToVideo,
}

/// The jobs Create offers, one tab each. A task can have several packs; Outpost
/// picks the one that suits the graphics card best (see [`pick`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Task {
    /// Text to image.
    Picture,
    /// Pictures of a trained character (a LoRA).
    Character,
    /// Change a picture.
    Edit,
    /// Text to video.
    Video,
    /// A picture brought to life (image to video).
    Animate,
}

impl Task {
    pub const ALL: [Task; 5] = [Task::Picture, Task::Character, Task::Edit, Task::Video, Task::Animate];

    fn of_lane(lane: Lane) -> Task {
        match lane {
            Lane::TextToImage => Task::Picture,
            Lane::EditImage => Task::Edit,
            Lane::TextToVideo => Task::Video,
            Lane::ImageToVideo => Task::Animate,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum OutputKind {
    Image,
    Video,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PackOutput {
    /// The node whose results are the pack's output (a SaveImage or video node).
    pub node: String,
    pub kind: OutputKind,
}

/// A model file the graph loads, and where ComfyUI expects it.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ModelFile {
    pub file: String,
    /// ComfyUI models subfolder: diffusion_models, text_encoders, vae, loras...
    pub folder: String,
    pub size_gb: f32,
    pub url: String,
    #[serde(default)]
    pub sha256: Option<String>,
    /// Exact size, when known; the download is checked against it.
    #[serde(default)]
    pub bytes: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum InputKind {
    /// Free text (prompts).
    Text,
    /// `WIDTHxHEIGHT` from `options`; sets two targets (width, then height).
    Size,
    Integer,
    Number,
    /// Empty for a random seed.
    Seed,
    /// One of `options`.
    Choice,
    /// An uploaded image (edit and image-to-video lanes).
    Image,
    /// A LoRA file from the LoRA folder; optional, inserts a loader node.
    Lora,
}

/// Where a chosen LoRA is spliced into the graph.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct LoraHook {
    /// The model output the LoRA loader reads, e.g. `["28", 0]`.
    #[schema(value_type = Vec<Object>)]
    pub model_from: (String, u32),
    /// Node inputs that should read the LoRA's model instead.
    #[schema(value_type = Vec<Vec<String>>)]
    pub model_into: Vec<(String, String)>,
    /// The input (key) that sets the LoRA's strength.
    #[serde(default)]
    pub strength_input: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct InputSpec {
    pub key: String,
    pub label: String,
    pub kind: InputKind,
    #[serde(default)]
    pub help: Option<String>,
    #[serde(default)]
    #[schema(value_type = Object)]
    pub default: Value,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub options: Vec<String>,
    #[serde(default)]
    pub min: Option<f64>,
    #[serde(default)]
    pub max: Option<f64>,
    #[serde(default)]
    pub step: Option<f64>,
    /// Shown under "More settings".
    #[serde(default)]
    pub advanced: bool,
    /// Node inputs this value is written to: `[node id, input name]`.
    #[serde(default)]
    #[schema(value_type = Vec<Vec<String>>)]
    pub targets: Vec<(String, String)>,
    #[serde(default)]
    pub lora: Option<LoraHook>,
    /// For a choice: what each option sets, as `[node, input, value]` triples
    /// (e.g. "Fast" sets 4 steps, CFG 1 and the Lightning LoRA's strength).
    #[serde(default)]
    #[schema(value_type = Object)]
    pub presets: std::collections::HashMap<String, Vec<(String, String, Value)>>,
    /// For an optional image: nodes to take out of the graph when none is given
    /// (with every link to them), e.g. the LoadImage of an unused reference picture.
    #[serde(default)]
    pub prune: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Pack {
    pub id: String,
    pub name: String,
    pub lane: Lane,
    /// The Create tab it belongs to. Left out, it follows the lane. (The API
    /// reports it as `task` beside the pack; see [`Pack::task`].)
    #[serde(default, skip_serializing)]
    pub task: Option<Task>,
    /// Higher is better: among the packs of a task that fit the card, the highest wins.
    #[serde(default)]
    pub rank: i32,
    pub description: String,
    #[serde(default)]
    pub license: Option<String>,
    /// Peak video memory the pack needs, roughly, in GB.
    pub vram_gb: f32,
    pub output: PackOutput,
    pub models: Vec<ModelFile>,
    pub inputs: Vec<InputSpec>,
    /// The ComfyUI graph in API format.
    #[serde(skip)]
    pub graph: Value,
}

/// A problem with what was asked for, shown to the user as is.
#[derive(Debug, thiserror::Error)]
pub enum PackError {
    #[error("{0}")]
    Invalid(String),
}

macro_rules! pack {
    ($id:literal) => {
        pack!($id, $id)
    };
    // A pack that reuses another pack's graph.
    ($id:literal, $graph:literal) => {
        (
            include_str!(concat!("../packs/", $id, "/pack.json")),
            include_str!(concat!("../packs/", $graph, "/graph.json")),
        )
    };
}

/// Built-in packs, in the order Create lists them within a task.
const BUILT_IN: &[(&str, &str)] = &[
    pack!("z_image_turbo"),
    pack!("qwen_image_21_turbo"),
    pack!("qwen_image"),
    pack!("character_z_image", "z_image_turbo"),
    pack!("qwen_image_21_edit"),
    pack!("qwen_image_edit"),
    pack!("wan22_t2v"),
    pack!("wan22_i2v"),
];

/// The packs that ship with Outpost.
pub fn built_in() -> Vec<Pack> {
    BUILT_IN
        .iter()
        .map(|(pack, graph)| parse(pack, graph).expect("built-in pack is valid"))
        .collect()
}

pub fn find(id: &str) -> Option<Pack> {
    built_in().into_iter().find(|p| p.id == id)
}

/// Read a pack and check that every target points at a real node input.
pub fn parse(pack_json: &str, graph_json: &str) -> Result<Pack, PackError> {
    let mut pack: Pack = serde_json::from_str(pack_json).map_err(|e| PackError::Invalid(format!("pack.json: {e}")))?;
    pack.graph = serde_json::from_str(graph_json).map_err(|e| PackError::Invalid(format!("graph.json: {e}")))?;
    pack.task.get_or_insert(Task::of_lane(pack.lane));
    for input in &pack.inputs {
        if !input.prune.is_empty() && (input.kind != InputKind::Image || input.required) {
            return Err(PackError::Invalid(format!(
                "{}: only an optional image can prune nodes",
                input.key
            )));
        }
        for node in &input.prune {
            if pack.graph.get(node).is_none() || *node == pack.output.node {
                return Err(PackError::Invalid(format!("{}: can't prune node {node}", input.key)));
            }
        }
    }
    let has_input = |node: &str, input: &str| {
        pack.graph
            .get(node)
            .and_then(|n| n.get("inputs"))
            .and_then(|i| i.get(input))
            .is_some()
    };
    for input in &pack.inputs {
        for (node, field) in &input.targets {
            if !has_input(node, field) {
                return Err(PackError::Invalid(format!(
                    "{}: {}.{} is not in the graph",
                    input.key, node, field
                )));
            }
        }
        if let Some(hook) = &input.lora {
            if pack.graph.get(&hook.model_from.0).is_none() {
                return Err(PackError::Invalid(format!(
                    "{}: node {} is not in the graph",
                    input.key, hook.model_from.0
                )));
            }
            for (node, field) in &hook.model_into {
                if !has_input(node, field) {
                    return Err(PackError::Invalid(format!(
                        "{}: {}.{} is not in the graph",
                        input.key, node, field
                    )));
                }
            }
        }
        for (option, sets) in &input.presets {
            if !input.options.contains(option) {
                return Err(PackError::Invalid(format!(
                    "{}: preset {option} isn't an option",
                    input.key
                )));
            }
            for (node, field, _) in sets {
                if !has_input(node, field) {
                    return Err(PackError::Invalid(format!(
                        "{}: {}.{} is not in the graph",
                        input.key, node, field
                    )));
                }
            }
        }
        if input.kind == InputKind::Size && input.targets.len() != 2 {
            return Err(PackError::Invalid(format!(
                "{}: a size sets exactly two targets",
                input.key
            )));
        }
    }
    if pack.graph.get(&pack.output.node).is_none() {
        return Err(PackError::Invalid(format!(
            "output node {} is not in the graph",
            pack.output.node
        )));
    }
    Ok(pack)
}

/// Seeds stay below 2^53 so JavaScript holds them exactly.
pub const MAX_SEED: u64 = 1 << 53;

/// The id given to the LoRA loader node Outpost adds.
pub const LORA_NODE: &str = "outpost_lora";

/// A graph ready to queue, and the seed it used (so a picture can be repeated).
#[derive(Debug, Clone, Default)]
pub struct Built {
    pub graph: Value,
    pub seed: Option<u64>,
}

impl Pack {
    /// Fill the graph from the user's values. `random_seed` supplies a seed when the
    /// user left it empty. `uploaded` maps image input keys to the names ComfyUI gave
    /// the uploaded files. `loras` is the list of LoRA files that exist.
    pub fn build(
        &self,
        values: &serde_json::Map<String, Value>,
        random_seed: impl Fn() -> u64,
        uploaded: &std::collections::HashMap<String, String>,
        loras: &[String],
    ) -> Result<Built, PackError> {
        let mut graph = self.graph.clone();
        let mut seed = None;
        for input in &self.inputs {
            let given = values.get(&input.key).filter(|v| !v.is_null() && *v != &json!(""));
            let value = match given {
                Some(v) => v.clone(),
                // Images come through `uploaded`; their own check is below.
                None if input.required && input.kind != InputKind::Image => {
                    return Err(PackError::Invalid(format!("{} is required.", input.label)));
                }
                None => input.default.clone(),
            };
            match input.kind {
                InputKind::Text => {
                    let text = value.as_str().unwrap_or_default();
                    if text.chars().count() > 4000 {
                        return Err(PackError::Invalid(format!(
                            "{} is too long (4,000 characters max).",
                            input.label
                        )));
                    }
                    set_all(&mut graph, &input.targets, json!(text));
                }
                InputKind::Size => {
                    let text = value.as_str().unwrap_or_default();
                    if !input.options.is_empty() && !input.options.iter().any(|o| o == text) {
                        return Err(PackError::Invalid(format!(
                            "{text} isn't one of the sizes for {}.",
                            input.label
                        )));
                    }
                    let (w, h) = parse_size(text)
                        .ok_or_else(|| PackError::Invalid(format!("{} must look like 1024x1024.", input.label)))?;
                    set(&mut graph, &input.targets[0], json!(w));
                    set(&mut graph, &input.targets[1], json!(h));
                }
                InputKind::Integer | InputKind::Number => {
                    let n = value
                        .as_f64()
                        .ok_or_else(|| PackError::Invalid(format!("{} must be a number.", input.label)))?;
                    if input.min.is_some_and(|m| n < m) || input.max.is_some_and(|m| n > m) {
                        return Err(PackError::Invalid(format!(
                            "{} must be between {} and {}.",
                            input.label,
                            input.min.unwrap_or(f64::MIN),
                            input.max.unwrap_or(f64::MAX)
                        )));
                    }
                    let v = if input.kind == InputKind::Integer {
                        json!(n.round() as i64)
                    } else {
                        json!(n)
                    };
                    set_all(&mut graph, &input.targets, v);
                }
                InputKind::Seed => {
                    let s = match &value {
                        Value::Null => random_seed(),
                        // Seeds must survive a round trip through JavaScript.
                        v => match v.as_u64() {
                            Some(s) if s < MAX_SEED => s,
                            _ => {
                                return Err(PackError::Invalid(format!(
                                    "{} must be a whole number below 9,007,199,254,740,992.",
                                    input.label
                                )))
                            }
                        },
                    };
                    seed = Some(s);
                    set_all(&mut graph, &input.targets, json!(s));
                }
                InputKind::Choice => {
                    let text = value.as_str().unwrap_or_default();
                    if !input.options.iter().any(|o| o == text) {
                        return Err(PackError::Invalid(format!(
                            "Pick one of the options for {}.",
                            input.label
                        )));
                    }
                    match input.presets.get(text) {
                        Some(sets) => {
                            for (node, field, v) in sets {
                                graph[node]["inputs"][field] = v.clone();
                            }
                        }
                        None => set_all(&mut graph, &input.targets, json!(text)),
                    }
                }
                InputKind::Image => match uploaded.get(&input.key) {
                    Some(name) => set_all(&mut graph, &input.targets, json!(name)),
                    None if input.required => {
                        return Err(PackError::Invalid(format!("Add an image for {}.", input.label)));
                    }
                    None => {
                        for node in &input.prune {
                            prune(&mut graph, node);
                        }
                    }
                },
                InputKind::Lora => {
                    let Some(name) = value.as_str().filter(|s| !s.is_empty()) else {
                        continue;
                    };
                    if !loras.iter().any(|l| l == name) {
                        return Err(PackError::Invalid(format!(
                            "The LoRA {name} isn't in your LoRA folder."
                        )));
                    }
                    let Some(hook) = &input.lora else { continue };
                    let strength = hook
                        .strength_input
                        .as_ref()
                        .and_then(|k| values.get(k).and_then(Value::as_f64).or_else(|| default_of(self, k)))
                        .unwrap_or(1.0)
                        .clamp(-2.0, 2.0);
                    graph[LORA_NODE] = json!({
                        "class_type": "LoraLoaderModelOnly",
                        "inputs": {
                            "model": [hook.model_from.0, hook.model_from.1],
                            "lora_name": name,
                            "strength_model": strength
                        }
                    });
                    for (node, field) in &hook.model_into {
                        graph[node]["inputs"][field] = json!([LORA_NODE, 0]);
                    }
                }
            }
        }
        Ok(Built { graph, seed })
    }
}

fn default_of(pack: &Pack, key: &str) -> Option<f64> {
    pack.inputs
        .iter()
        .find(|i| i.key == key)
        .and_then(|i| i.default.as_f64())
}

/// Take a node out of the graph, and every input that reads from it.
fn prune(graph: &mut Value, id: &str) {
    let Some(nodes) = graph.as_object_mut() else { return };
    nodes.remove(id);
    for node in nodes.values_mut() {
        if let Some(inputs) = node.get_mut("inputs").and_then(Value::as_object_mut) {
            inputs.retain(|_, v| v.get(0).and_then(Value::as_str) != Some(id) || !v.is_array());
        }
    }
}

impl Pack {
    pub fn task(&self) -> Task {
        self.task.unwrap_or(Task::of_lane(self.lane))
    }

    /// Every ComfyUI node type the pack can use, sorted.
    pub fn node_types(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .graph
            .as_object()
            .into_iter()
            .flat_map(|g| g.values())
            .filter_map(|n| n.get("class_type").and_then(Value::as_str).map(str::to_string))
            .collect();
        if self.inputs.iter().any(|i| i.kind == InputKind::Lora) {
            out.push("LoraLoaderModelOnly".into());
        }
        out.sort();
        out.dedup();
        out
    }

    /// The node types it needs that a ComfyUI with `have` lacks.
    pub fn missing_nodes(&self, have: &std::collections::HashSet<String>) -> Vec<String> {
        self.node_types().into_iter().filter(|n| !have.contains(n)).collect()
    }
}

/// Does a pack needing `vram_gb` at its peak run on a card with `card_gb` and
/// `ram_gb` of system memory? ComfyUI can spill to system memory, so between the
/// two it runs, slowly.
pub fn fit(vram_gb: f32, card_gb: f32, ram_gb: f32) -> Fit {
    if card_gb > 0.0 && vram_gb <= card_gb * 0.98 {
        Fit::Fits
    } else if vram_gb <= card_gb + ram_gb * 0.5 {
        Fit::Partial
    } else {
        Fit::TooLarge
    }
}

/// The pack Outpost suggests for a task: the best-ranked one that fits the card;
/// failing that, the smallest that runs at all. `None` when nothing will run.
pub fn pick<'a>(packs: &'a [Pack], task: Task, fit: impl Fn(&Pack) -> Fit) -> Option<&'a Pack> {
    let of_task = || packs.iter().filter(move |p| p.task() == task);
    // Ties go to the pack listed first.
    of_task()
        .filter(|p| fit(p) == Fit::Fits)
        .fold(None::<&Pack>, |best, p| match best {
            Some(b) if b.rank >= p.rank => Some(b),
            _ => Some(p),
        })
        .or_else(|| {
            of_task()
                .filter(|p| fit(p) == Fit::Partial)
                .fold(None::<&Pack>, |best, p| match best {
                    Some(b) if b.vram_gb <= p.vram_gb => Some(b),
                    _ => Some(p),
                })
        })
}

fn set(graph: &mut Value, (node, field): &(String, String), value: Value) {
    graph[node]["inputs"][field] = value;
}

fn set_all(graph: &mut Value, targets: &[(String, String)], value: Value) {
    for t in targets {
        set(graph, t, value.clone());
    }
}

/// `"1024x768"` → (1024, 768). Both sides 256 to 2048, multiples of 16.
pub fn parse_size(s: &str) -> Option<(u32, u32)> {
    let (w, h) = s.split_once(['x', 'X', '×'])?;
    let (w, h): (u32, u32) = (w.trim().parse().ok()?, h.trim().parse().ok()?);
    let ok = |n: u32| (256..=2048).contains(&n) && n % 16 == 0;
    (ok(w) && ok(h)).then_some((w, h))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn values(v: Value) -> serde_json::Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn built_in_packs_are_valid() {
        let packs = built_in();
        assert!(!packs.is_empty());
        for p in &packs {
            assert!(!p.models.is_empty(), "{} lists its models", p.id);
            assert!(
                p.models.iter().all(|m| m.url.starts_with("https://")),
                "{} model urls",
                p.id
            );
            assert!(p.models.iter().all(|m| !m.file.contains('/') && !m.file.contains('\\')));
            // Every file is pinned, so a damaged or mixed-up download is caught.
            assert!(
                p.models
                    .iter()
                    .all(|m| m.sha256.as_ref().is_some_and(|h| h.len() == 64) && m.bytes.is_some()),
                "{} pins its files",
                p.id
            );
        }
    }

    #[test]
    fn z_image_fills_prompt_size_steps_and_seed() {
        let p = find("z_image_turbo").unwrap();
        let built = p
            .build(
                &values(json!({"prompt": "a lighthouse at dusk", "size": "832x1216", "steps": 6})),
                || 42,
                &HashMap::new(),
                &[],
            )
            .unwrap();
        let g = &built.graph;
        assert_eq!(g["27"]["inputs"]["text"], "a lighthouse at dusk");
        assert_eq!(g["13"]["inputs"]["width"], 832);
        assert_eq!(g["13"]["inputs"]["height"], 1216);
        assert_eq!(g["3"]["inputs"]["steps"], 6);
        assert_eq!(g["3"]["inputs"]["seed"], 42);
        assert_eq!(built.seed, Some(42));
        // No LoRA chosen: the graph is untouched there.
        assert!(g.get(LORA_NODE).is_none());
        assert_eq!(g["11"]["inputs"]["model"], json!(["28", 0]));
    }

    #[test]
    fn lora_is_spliced_in_with_its_strength() {
        let p = find("z_image_turbo").unwrap();
        let loras = vec!["d4nni.safetensors".to_string()];
        let built = p
            .build(
                &values(json!({"prompt": "d4nni at the beach", "lora": "d4nni.safetensors", "lora_strength": 0.8, "seed": 7})),
                || 1,
                &HashMap::new(),
                &loras,
            )
            .unwrap();
        let g = &built.graph;
        assert_eq!(g[LORA_NODE]["class_type"], "LoraLoaderModelOnly");
        assert_eq!(g[LORA_NODE]["inputs"]["model"], json!(["28", 0]));
        assert_eq!(g[LORA_NODE]["inputs"]["strength_model"], 0.8);
        assert_eq!(g["11"]["inputs"]["model"], json!([LORA_NODE, 0]));
        assert_eq!(g["3"]["inputs"]["seed"], 7);

        // Only LoRAs that exist; never a path.
        let bad = p.build(
            &values(json!({"prompt": "x", "lora": "../../secret.safetensors"})),
            || 1,
            &HashMap::new(),
            &loras,
        );
        assert!(matches!(bad, Err(PackError::Invalid(_))));
    }

    #[test]
    fn bad_values_are_refused() {
        let p = find("z_image_turbo").unwrap();
        let none = HashMap::new();
        for v in [
            json!({}),
            json!({"prompt": ""}),
            json!({"prompt": "x", "size": "999x999"}),
            json!({"prompt": "x", "steps": 500}),
            json!({"prompt": "x", "steps": "lots"}),
        ] {
            assert!(p.build(&values(v.clone()), || 1, &none, &[]).is_err(), "{v}");
        }
    }

    #[test]
    fn presets_set_several_values() {
        let p = find("qwen_image").unwrap();
        let none = HashMap::new();
        let fast = p
            .build(
                &values(json!({"prompt": "a poster that says \"OPEN\""})),
                || 1,
                &none,
                &[],
            )
            .unwrap()
            .graph;
        assert_eq!(fast["3"]["inputs"]["steps"], 8);
        assert_eq!(fast["73"]["inputs"]["strength_model"], 1);
        let quality = p
            .build(
                &values(json!({"prompt": "x", "speed": "Quality (20 steps)"})),
                || 1,
                &none,
                &[],
            )
            .unwrap()
            .graph;
        assert_eq!(quality["3"]["inputs"]["steps"], 20);
        assert_eq!(quality["3"]["inputs"]["cfg"], 4);
        assert_eq!(quality["73"]["inputs"]["strength_model"], 0);
        assert!(p
            .build(&values(json!({"prompt": "x", "speed": "Turbo"})), || 1, &none, &[])
            .is_err());

        let wan = find("wan22_t2v").unwrap();
        let g = wan
            .build(
                &values(json!({"prompt": "waves", "length": "2 seconds"})),
                || 1,
                &none,
                &[],
            )
            .unwrap()
            .graph;
        assert_eq!(g["55"]["inputs"]["length"], 49);
        assert!(g["55"]["inputs"].get("start_image").is_none());
    }

    #[test]
    fn image_inputs_need_an_upload() {
        let p = find("qwen_image_edit").unwrap();
        let v = values(json!({"prompt": "make it night"}));
        assert!(p.build(&v, || 1, &HashMap::new(), &[]).is_err());
        let mut up = HashMap::new();
        up.insert("image".to_string(), "outpost/abc.png".to_string());
        let g = p.build(&v, || 1, &up, &[]).unwrap().graph;
        assert_eq!(g["78"]["inputs"]["image"], "outpost/abc.png");
        assert_eq!(g["111"]["inputs"]["prompt"], "make it night");
        assert_eq!(g["3"]["inputs"]["steps"], 4);

        let i2v = find("wan22_i2v").unwrap();
        let g = i2v
            .build(&values(json!({"prompt": "she smiles"})), || 1, &up, &[])
            .unwrap()
            .graph;
        assert_eq!(g["55"]["inputs"]["start_image"], json!(["56", 0]));
        assert_eq!(g["56"]["inputs"]["image"], "outpost/abc.png");
    }

    #[test]
    fn every_pack_has_a_prompt_and_a_seed() {
        for p in built_in() {
            assert!(p.inputs.iter().any(|i| i.key == "prompt" && i.required), "{}", p.id);
            assert!(p.inputs.iter().any(|i| i.kind == InputKind::Seed), "{}", p.id);
            // Every graph saves somewhere Outpost reads from.
            assert!(p.graph[&p.output.node]["inputs"]["filename_prefix"]
                .as_str()
                .unwrap()
                .starts_with("outpost/"));
        }
    }

    #[test]
    fn seeds_must_fit_javascript() {
        let p = find("z_image_turbo").unwrap();
        let none = HashMap::new();
        assert!(p
            .build(
                &values(json!({"prompt": "x", "seed": 9_007_199_254_740_991u64})),
                || 1,
                &none,
                &[]
            )
            .is_ok());
        assert!(p
            .build(
                &values(json!({"prompt": "x", "seed": 9_007_199_254_740_992u64})),
                || 1,
                &none,
                &[]
            )
            .is_err());
        assert!(p
            .build(&values(json!({"prompt": "x", "seed": -5})), || 1, &none, &[])
            .is_err());
        assert!(p
            .build(&values(json!({"prompt": "x", "seed": 1.5})), || 1, &none, &[])
            .is_err());
    }

    #[test]
    fn sizes_parse() {
        assert_eq!(parse_size("1024x768"), Some((1024, 768)));
        assert_eq!(parse_size("1024 × 1024"), Some((1024, 1024)));
        assert_eq!(parse_size("1000x1000"), None);
        assert_eq!(parse_size("4096x4096"), None);
        assert_eq!(parse_size("big"), None);
    }

    #[test]
    fn qwen_21_edit_drops_unused_references() {
        let p = find("qwen_image_21_edit").unwrap();
        let mut up = HashMap::new();
        up.insert("image".to_string(), "outpost/a.png".to_string());
        let v = values(json!({"prompt": "make it night", "seed": 5}));
        let g = p.build(&v, || 1, &up, &[]).unwrap().graph;
        assert_eq!(g["470"]["inputs"]["image"], "outpost/a.png");
        assert!(g.get("471").is_none() && g.get("472").is_none());
        let enc = &g["474"]["inputs"];
        assert_eq!(enc["images.image_1"], json!(["470", 0]));
        assert!(enc.get("images.image_2").is_none() && enc.get("images.image_3").is_none());
        assert_eq!(enc["resolution"], 1024);
        assert_eq!(g["504"]["inputs"]["noise_seed"], 5);
        // Nothing left pointing at a removed node.
        for node in g.as_object().unwrap().values() {
            for input in node["inputs"].as_object().unwrap().values() {
                if let Some(from) = input.get(0).and_then(Value::as_str) {
                    assert!(g.get(from).is_some(), "dangling link to {from}");
                }
            }
        }

        up.insert("ref2".to_string(), "outpost/b.png".to_string());
        let v = values(json!({"prompt": "she wears <image2>", "detail": "Same as the picture"}));
        let g = p.build(&v, || 1, &up, &[]).unwrap().graph;
        assert_eq!(g["474"]["inputs"]["images.image_2"], json!(["471", 0]));
        assert_eq!(g["471"]["inputs"]["image"], "outpost/b.png");
        assert!(g.get("472").is_none());
        assert_eq!(g["474"]["inputs"]["resolution"], 0);

        // The picture to edit is still required.
        assert!(p.build(&v, || 1, &HashMap::new(), &[]).is_err());
    }

    #[test]
    fn character_needs_a_lora() {
        let p = find("character_z_image").unwrap();
        assert_eq!(p.task(), Task::Character);
        let loras = vec!["zd4nni.safetensors".to_string()];
        let none = HashMap::new();
        assert!(p.build(&values(json!({"prompt": "zd4nni on a beach"})), || 1, &none, &loras).is_err());
        let g = p
            .build(
                &values(json!({"prompt": "zd4nni on a beach", "lora": "zd4nni.safetensors"})),
                || 1,
                &none,
                &loras,
            )
            .unwrap()
            .graph;
        assert_eq!(g[LORA_NODE]["inputs"]["lora_name"], "zd4nni.safetensors");
        // It shares Z-Image Turbo's graph and files.
        assert_eq!(p.graph, find("z_image_turbo").unwrap().graph);
    }

    #[test]
    fn every_task_has_a_pack_and_ids_are_unique() {
        let packs = built_in();
        for t in Task::ALL {
            assert!(packs.iter().any(|p| p.task() == t), "{t:?}");
        }
        let mut ids: Vec<_> = packs.iter().map(|p| p.id.as_str()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), packs.len());
        // A file shared by two packs is described the same way in both.
        let mut seen: HashMap<String, (String, Option<String>, Option<u64>)> = HashMap::new();
        for p in &packs {
            for m in &p.models {
                let key = format!("{}/{}", m.folder, m.file);
                let v = (m.url.clone(), m.sha256.clone(), m.bytes);
                if let Some(old) = seen.insert(key.clone(), v.clone()) {
                    assert_eq!(old, v, "{key}");
                }
            }
        }
    }

    #[test]
    fn picks_suit_the_card() {
        let packs = built_in();
        let pick_on = |card: f32, ram: f32, task: Task| {
            pick(&packs, task, |p| fit(p.vram_gb, card, ram)).map(|p| p.id.clone())
        };
        // 16 GB card, 64 GB RAM: the best that fits.
        assert_eq!(pick_on(16.0, 64.0, Task::Picture).as_deref(), Some("z_image_turbo"));
        assert_eq!(pick_on(16.0, 64.0, Task::Edit).as_deref(), Some("qwen_image_21_edit"));
        assert_eq!(pick_on(16.0, 64.0, Task::Character).as_deref(), Some("character_z_image"));
        // 12 GB: Z-Image (14 GB) no longer fits, so the 11 GB Qwen 2.1 Turbo wins.
        assert_eq!(pick_on(12.0, 32.0, Task::Picture).as_deref(), Some("qwen_image_21_turbo"));
        // 8 GB laptop: nothing fits, so the smallest that runs.
        assert_eq!(pick_on(8.0, 16.0, Task::Picture).as_deref(), Some("qwen_image_21_turbo"));
        assert_eq!(pick_on(8.0, 16.0, Task::Character).as_deref(), Some("character_z_image"));
        // 48 GB: still the best-ranked, not merely the biggest.
        assert_eq!(pick_on(48.0, 128.0, Task::Edit).as_deref(), Some("qwen_image_21_edit"));
        // No GPU and little memory: nothing.
        assert_eq!(pick_on(0.0, 8.0, Task::Edit), None);
    }

    #[test]
    fn packs_know_the_nodes_they_need() {
        let edit = find("qwen_image_21_edit").unwrap();
        let nodes = edit.node_types();
        assert!(nodes.contains(&"TextEncodeQwenImage21".to_string()));
        assert!(nodes.contains(&"SaveImage".to_string()));
        let z = find("z_image_turbo").unwrap();
        assert!(z.node_types().contains(&"LoraLoaderModelOnly".to_string()));

        let old: std::collections::HashSet<String> = edit
            .node_types()
            .into_iter()
            .filter(|n| !n.starts_with("TextEncodeQwenImage21") && n != "QwenImage21Cache")
            .collect();
        assert_eq!(edit.missing_nodes(&old), vec!["QwenImage21Cache", "TextEncodeQwenImage21"]);
        let all: std::collections::HashSet<String> = edit.node_types().into_iter().collect();
        assert!(edit.missing_nodes(&all).is_empty());
    }

    #[test]
    fn broken_packs_are_caught() {
        let graph = r#"{"1": {"class_type": "SaveImage", "inputs": {"images": ["2", 0]}}}"#;
        let pack = r#"{"id":"x","name":"X","lane":"text_to_image","description":"","vram_gb":1,
            "output":{"node":"1","kind":"image"},"models":[],
            "inputs":[{"key":"p","label":"P","kind":"text","targets":[["9","text"]]}]}"#;
        assert!(parse(pack, graph).is_err());
    }
}
