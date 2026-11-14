//! `lumalink add character <model.glb> --replaces mario|luigi|<model>`: a rigged character from any tool
//! made into a model that stands in for one of the game's, so the game's animations move it.
//!
//! 1. **Map** each of the game model's joints to one of the source's: by name (`ArmL1` = `Arm.L.001`...), then by a
//!    synonym table (Mixamo, VRM, Rigify, Unreal, the game's own: `LeftForeArm`, `J_Bip_L_LowerArm`,
//!    `forearm.L`, `lowerarm_l`, `ArmL2` are one part), then by position under the nearest mapped ancestor, with a
//!    score. Each game joint is reported mapped (how), guessed (score) or missing (no part of the source follows it).
//! 2. **Rescale** the source to the game character's height (the skeletons' heights; glTF metres, 100 game units each).
//! 3. **Rebind**: the model's skeleton becomes the game's own (every joint, its name and its rest pose, as
//!    docs/modding/content/models.md, section 4 wants), and the mesh is bent into that rest pose: each mapped joint's bone turned onto the game's bone
//!    and moved to the game's joint, the vertices following their weights. A source joint the game lacks (hair, tail,
//!    fingers) stays as an extra joint under its mapped ancestor; its weights stay with it.
//! 4. **T-pose check**: a bone turned more than [`BENT_DEGREES`] to meet the game's is named in the report (the
//!    source was not in the game's rest pose there, and the mesh is bent there).
//!
//! The result is a .glb at the game model's place in the mod's base layer (docs/modding/content/models.md, section 2); the cook (3.2) makes
//! the game's model block of it as of any replacing model. The source's own animations are left out (they move its
//! old joints) and counted in the report.

use crate::say;
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::Path;

/// A bone turned more than this to meet the game's rest pose is named in the report.
pub const BENT_DEGREES: f32 = 20.0;
/// A position guess at or above this score is used; below it the game joint is missing.
pub const GUESS_SCORE: f32 = 0.6;
/// Fewer mapped joints than this is no character of the game's shape: refused.
pub const FEWEST_MAPPED: usize = 3;
const UNITS_PER_METRE: f32 = 100.0;

type Mat = [f32; 16];
type V3 = [f32; 3];
type Quat = [f32; 4];

const IDENTITY: Mat = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0];

// ------------------------------------------------------------------ small math (column-major, as glTF)

fn mul(a: &Mat, b: &Mat) -> Mat {
    let mut o = [0.0f32; 16];
    for c in 0..4 {
        for r in 0..4 {
            o[c * 4 + r] = (0..4).map(|k| a[k * 4 + r] * b[c * 4 + k]).sum();
        }
    }
    o
}

fn point(m: &Mat, p: V3) -> V3 {
    [0, 1, 2].map(|r| m[r] * p[0] + m[4 + r] * p[1] + m[8 + r] * p[2] + m[12 + r])
}

fn vector(m: &Mat, v: V3) -> V3 {
    [0, 1, 2].map(|r| m[r] * v[0] + m[4 + r] * v[1] + m[8 + r] * v[2])
}

/// An affine matrix's inverse; the identity for a singular one.
fn inverse(m: &Mat) -> Mat {
    let a = |r: usize, c: usize| f64::from(m[c * 4 + r]);
    let det = a(0, 0) * (a(1, 1) * a(2, 2) - a(1, 2) * a(2, 1)) - a(0, 1) * (a(1, 0) * a(2, 2) - a(1, 2) * a(2, 0))
        + a(0, 2) * (a(1, 0) * a(2, 1) - a(1, 1) * a(2, 0));
    if det.abs() < 1e-18 {
        return IDENTITY;
    }
    let i = [
        [(a(1, 1) * a(2, 2) - a(1, 2) * a(2, 1)) / det, (a(0, 2) * a(2, 1) - a(0, 1) * a(2, 2)) / det, (a(0, 1) * a(1, 2) - a(0, 2) * a(1, 1)) / det],
        [(a(1, 2) * a(2, 0) - a(1, 0) * a(2, 2)) / det, (a(0, 0) * a(2, 2) - a(0, 2) * a(2, 0)) / det, (a(0, 2) * a(1, 0) - a(0, 0) * a(1, 2)) / det],
        [(a(1, 0) * a(2, 1) - a(1, 1) * a(2, 0)) / det, (a(0, 1) * a(2, 0) - a(0, 0) * a(2, 1)) / det, (a(0, 0) * a(1, 1) - a(0, 1) * a(1, 0)) / det],
    ];
    let t = [a(0, 3), a(1, 3), a(2, 3)];
    let mut out = IDENTITY;
    for r in 0..3 {
        for c in 0..3 {
            out[c * 4 + r] = i[r][c] as f32;
        }
        out[12 + r] = -(i[r][0] * t[0] + i[r][1] * t[1] + i[r][2] * t[2]) as f32;
    }
    out
}

fn quat_mat(q: Quat) -> Mat {
    let [x, y, z, w] = q;
    [
        1.0 - 2.0 * (y * y + z * z), 2.0 * (x * y + z * w), 2.0 * (x * z - y * w), 0.0,
        2.0 * (x * y - z * w), 1.0 - 2.0 * (x * x + z * z), 2.0 * (y * z + x * w), 0.0,
        2.0 * (x * z + y * w), 2.0 * (y * z - x * w), 1.0 - 2.0 * (x * x + y * y), 0.0,
        0.0, 0.0, 0.0, 1.0,
    ]
}

fn quat_mul(a: Quat, b: Quat) -> Quat {
    let [ax, ay, az, aw] = a;
    let [bx, by, bz, bw] = b;
    [aw * bx + ax * bw + ay * bz - az * by, aw * by - ax * bz + ay * bw + az * bx, aw * bz + ax * by - ay * bx + az * bw, aw * bw - ax * bx - ay * by - az * bz]
}

/// The game's joint angles (X, Y, Z radians, composed Z·Y·X) as a quaternion.
fn euler_zyx(r: V3) -> Quat {
    let axis = |i: usize, angle: f32| {
        let mut q = [0.0, 0.0, 0.0, (angle / 2.0).cos()];
        q[i] = (angle / 2.0).sin();
        q
    };
    quat_mul(quat_mul(axis(2, r[2]), axis(1, r[1])), axis(0, r[0]))
}

fn trs(t: V3, q: Quat, s: V3) -> Mat {
    let mut m = quat_mat(q);
    for c in 0..3 {
        for r in 0..3 {
            m[c * 4 + r] *= s[c];
        }
    }
    m[12] = t[0];
    m[13] = t[1];
    m[14] = t[2];
    m
}

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn length(a: V3) -> f32 {
    (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt()
}

fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

/// The shortest turn taking direction `from` onto `to`.
fn arc(from: V3, to: V3) -> Quat {
    let (lf, lt) = (length(from), length(to));
    if lf < 1e-6 || lt < 1e-6 {
        return [0.0, 0.0, 0.0, 1.0];
    }
    let (f, t) = (from.map(|v| v / lf), to.map(|v| v / lt));
    let d = f[0] * t[0] + f[1] * t[1] + f[2] * t[2];
    if d < -0.999_999 {
        // Opposite: half a turn about any axis square to `from`.
        let mut axis = cross([1.0, 0.0, 0.0], f);
        if length(axis) < 1e-3 {
            axis = cross([0.0, 1.0, 0.0], f);
        }
        let l = length(axis);
        return [axis[0] / l, axis[1] / l, axis[2] / l, 0.0];
    }
    let c = cross(f, t);
    let q = [c[0], c[1], c[2], 1.0 + d];
    let n = q.iter().map(|v| v * v).sum::<f32>().sqrt();
    q.map(|v| v / n)
}

fn quat_degrees(q: Quat) -> f32 {
    2.0 * q[3].abs().clamp(0.0, 1.0).acos().to_degrees()
}

// ------------------------------------------------------------------ the game's skeleton

/// The skeleton of the game model a character stands in for: names, parents and rest pose (in game units), as the
/// store's model block holds them (nebula_model.h NebulaModelJoint).
#[derive(Clone, Debug)]
pub struct GameSkeleton {
    /// Its path as a mod's layer names it (`ObjectData/Mario.arc/Mario.bdl`).
    pub path: String,
    pub names: Vec<String>,
    pub parents: Vec<Option<usize>>,
    /// Each joint's rest transform from its parent, translations in game units.
    pub local: Vec<Mat>,
}

impl GameSkeleton {
    /// The skeleton of a model block; None when `block` is no model block.
    pub fn read(path: &str, block: &[u8]) -> Option<GameSkeleton> {
        let model = nebula_import::cook::gltf::GameModel::read(path, block)?;
        let word = |at: usize| Some(u32::from_le_bytes(block.get(at..at.checked_add(4)?)?.try_into().ok()?) as usize);
        let float = |at: usize| Some(f32::from_le_bytes(block.get(at..at.checked_add(4)?)?.try_into().ok()?));
        let joints_at = word(15 * 4)?;
        let bytes = nebula_import::cook::model_write::JOINT_BYTES;
        let mut local = Vec::with_capacity(model.joints.len());
        for j in 0..model.joints.len() {
            let at = joints_at.checked_add(j * bytes)?;
            let three = |o: usize| -> Option<V3> { Some([float(at + o)?, float(at + o + 4)?, float(at + o + 8)?]) };
            let (scale, rotation, translation) = (three(0)?, three(12)?, three(28)?);
            local.push(trs(translation, euler_zyx(rotation), scale));
        }
        Some(GameSkeleton { path: path.to_string(), names: model.joints, parents: model.parents, local })
    }

    /// Each joint's rest transform in the model's space (game units).
    pub fn world(&self) -> Vec<Mat> {
        let mut out: Vec<Option<Mat>> = vec![None; self.names.len()];
        for j in 0..self.names.len() {
            world_of(j, &self.parents, &self.local, &mut out, 0);
        }
        out.into_iter().map(|m| m.unwrap_or(IDENTITY)).collect()
    }
}

fn world_of(j: usize, parents: &[Option<usize>], local: &[Mat], out: &mut Vec<Option<Mat>>, depth: usize) -> Mat {
    if let Some(m) = out[j] {
        return m;
    }
    let m = match parents.get(j).copied().flatten() {
        // A parent loop in a broken block ends at the depth of the skeleton.
        Some(p) if depth < parents.len() => mul(&world_of(p, parents, local, out, depth + 1), &local[j]),
        _ => local[j],
    };
    out[j] = Some(m);
    m
}

/// The game model `replaces` names (`mario`, `luigi`, or any model's name): its path as a mod's layer names it and its
/// block. The model named exactly `<name>.bdl` or `.bmd` in the archive of that name comes first, then any of that name.
pub fn find_model(store: &Path, replaces: &str) -> Result<(String, Vec<u8>)> {
    use std::io::{Read, Seek, SeekFrom};
    let wanted = replaces.to_ascii_lowercase();
    let files = crate::game_store::GameStore::disc_files(store)?;
    let stem = |path: &str| path.rsplit('/').next().unwrap_or(path).rsplit_once('.').map(|(s, e)| (s.to_ascii_lowercase(), e.to_ascii_lowercase()));
    let models: Vec<&(String, u64, u64)> = files.iter().filter(|(p, _, _)| stem(p).is_some_and(|(s, e)| s == wanted && (e == "bdl" || e == "bmd"))).collect();
    let in_own = models.iter().find(|(p, _, _)| p.to_ascii_lowercase().contains(&format!("/{wanted}.arc/")));
    let Some((path, position, size)) = in_own.or(models.first()).copied() else {
        bail!(say!("LumaLink.Character.NoGameModel", name = replaces));
    };
    let mut pak = std::fs::File::open(store).with_context(|| say!("LumaLink.CannotRead", path = store.display()))?;
    let mut bytes = vec![0u8; usize::try_from(*size)?];
    pak.seek(SeekFrom::Start(*position))?;
    pak.read_exact(&mut bytes)?;
    Ok((path.clone(), bytes))
}

// ------------------------------------------------------------------ names

/// A body part, whatever a tool names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Part {
    Root,
    Pelvis,
    Spine,
    Chest,
    Neck,
    Head,
    /// A cap, hat or helmet: the game's CapPosition (Mario's and Luigi's caps), SMO's Cap.
    Hat,
    Shoulder,
    UpperArm,
    LowerArm,
    Hand,
    UpperLeg,
    LowerLeg,
    Foot,
    Toe,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Side {
    Centre,
    Left,
    Right,
}

/// A joint name taken apart: its words without the tool's prefixes and its side, its side, its number.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NameParts {
    pub base: String,
    pub side: Side,
    pub index: Option<u32>,
}

/// Words the tools put in names that say nothing of the part (Mixamo's namespace, VRM's `J_Bip_C_`, Rigify's `DEF-`,
/// Biped's `Bip01`, Character Creator's `CC_Base_`).
const NOISE: [&str; 14] = ["mixamorig", "j", "bip", "c", "def", "org", "mch", "bip01", "bip001", "cc", "base", "jnt", "joint", "bone"];

/// Takes a joint name apart (`mixamorig:LeftForeArm`, `J_Bip_L_LowerArm`, `forearm.L`, `lowerarm_l`, `ArmL2`).
pub fn name_parts(name: &str) -> NameParts {
    // A namespace (`mixamorig:`, `Armature|`) is no part of the name.
    let name = name.rsplit([':', '|']).next().unwrap_or(name);
    let mut words: Vec<String> = Vec::new();
    let mut word = String::new();
    let mut last: Option<char> = None;
    for c in name.chars() {
        let split = match last {
            None => false,
            Some(l) => !c.is_alphanumeric() || (l.is_lowercase() && c.is_uppercase()) || (l.is_alphabetic() != c.is_alphabetic() && l.is_alphanumeric()),
        };
        if split && !word.is_empty() {
            words.push(std::mem::take(&mut word).to_lowercase());
        }
        if c.is_alphanumeric() {
            word.push(c);
        }
        last = Some(c);
    }
    if !word.is_empty() {
        words.push(word.to_lowercase());
    }
    let mut side = Side::Centre;
    let mut index = None;
    let mut kept = Vec::new();
    for w in words {
        match w.as_str() {
            "left" | "l" => side = Side::Left,
            "right" | "r" => side = Side::Right,
            _ if w.chars().all(|c| c.is_ascii_digit()) => index = w.parse().ok(),
            _ if NOISE.contains(&w.as_str()) => {}
            _ => kept.push(w),
        }
    }
    NameParts { base: kept.concat(), side, index }
}

/// The part a name stands for, by the synonym table.
pub fn part(p: &NameParts) -> Option<Part> {
    Some(match p.base.as_str() {
        "center" | "centre" | "root" | "cog" => Part::Root,
        "hips" | "hip" | "pelvis" | "waist" => Part::Pelvis,
        "spine" | "abdomen" | "torso" | "lowerback" => {
            if p.index.is_some_and(|i| i >= 2) {
                Part::Chest
            } else {
                Part::Spine
            }
        }
        "chest" | "upperchest" | "thorax" | "ribcage" | "spineupper" => Part::Chest,
        "neck" => Part::Neck,
        "head" => Part::Head,
        "cap" | "hat" | "helmet" | "capposition" | "hatposition" => Part::Hat,
        "shoulder" | "clavicle" | "collar" | "clav" | "collarbone" => Part::Shoulder,
        "upperarm" | "uparm" | "bicep" => Part::UpperArm,
        "arm" => {
            if p.index == Some(2) {
                Part::LowerArm
            } else {
                Part::UpperArm
            }
        }
        "forearm" | "lowerarm" | "elbow" => Part::LowerArm,
        "hand" | "wrist" => Part::Hand,
        "upleg" | "thigh" | "upperleg" => Part::UpperLeg,
        "leg" => {
            if p.index == Some(1) {
                Part::UpperLeg
            } else {
                Part::LowerLeg
            }
        }
        "calf" | "shin" | "knee" | "lowerleg" | "crus" => Part::LowerLeg,
        "foot" | "ankle" => Part::Foot,
        "toe" | "toebase" | "toes" | "ball" => Part::Toe,
        _ => return None,
    })
}

/// How a game joint found its source joint.
#[derive(Clone, Debug, PartialEq)]
pub enum How {
    /// The maker's own choice (`--joint GAME=SOURCE`).
    Given,
    Name,
    Synonym,
    /// By position under its mapped ancestor, with the score (1 at the same place).
    Position(f32),
}

/// What each game joint got: its source joint and how, or none.
#[derive(Clone, Debug, PartialEq)]
pub struct Mapped {
    pub source: Vec<Option<(usize, How)>>,
}

/// A maker's choices for [`map_joints`] (`--joint GAME=SOURCE`): each game joint's number and its source joint's,
/// none for a game joint that follows no part of the model (`GAME=`).
pub type Given = Vec<(usize, Option<usize>)>;

/// `--joint` choices by name, against the game's joints and the source's: an unknown name, or a joint named twice,
/// refused naming it.
pub fn given_joints(choices: &[(String, String)], game: &GameSkeleton, source_names: &[String]) -> Result<Given> {
    let mut out: Given = Vec::new();
    for (g, s) in choices {
        let gi = game.names.iter().position(|n| n == g).ok_or_else(|| anyhow::anyhow!(say!("LumaLink.Character.NoGameJoint", joint = g, model = game.path)))?;
        let si = if s.is_empty() {
            None
        } else {
            Some(source_names.iter().position(|n| n == s).ok_or_else(|| anyhow::anyhow!(say!("LumaLink.Character.NoSourceJoint", joint = s)))?)
        };
        if out.iter().any(|(o, os)| *o == gi || (si.is_some() && *os == si)) {
            bail!(say!("LumaLink.Character.JointTwice", joint = format!("{g}={s}")));
        }
        out.push((gi, si));
    }
    Ok(out)
}

/// Maps the game's joints to the source's (`source_world`: each source joint's rest place in its model, metres):
/// first the maker's `given` choices, then by name, by synonym and by position for the rest.
pub fn map_joints(game: &GameSkeleton, source_names: &[String], source_parents: &[Option<usize>], source_world: &[V3], given: &Given) -> Mapped {
    let gp: Vec<NameParts> = game.names.iter().map(|n| name_parts(n)).collect();
    let sp: Vec<NameParts> = source_names.iter().map(|n| name_parts(n)).collect();
    let mut source: Vec<Option<(usize, How)>> = vec![None; game.names.len()];
    let mut taken = vec![false; source_names.len()];
    // 0. The maker's: kept whatever the steps below would find (a game joint given none stays none).
    let mut fixed = vec![false; game.names.len()];
    for &(g, s) in given {
        fixed[g] = true;
        if let Some(s) = s {
            taken[s] = true;
            source[g] = Some((s, How::Given));
        }
    }
    // 1. By name: the same words, side and number.
    for (g, p) in gp.iter().enumerate() {
        if p.base.is_empty() || fixed[g] {
            continue;
        }
        if let Some(s) = (0..sp.len()).find(|&s| !taken[s] && sp[s] == *p) {
            taken[s] = true;
            source[g] = Some((s, How::Name));
        }
    }
    // 2. By the synonym table: the same part and side.
    for (g, p) in gp.iter().enumerate() {
        if source[g].is_some() || fixed[g] {
            continue;
        }
        let Some(part_g) = part(p) else { continue };
        if let Some(s) = (0..sp.len()).find(|&s| !taken[s] && sp[s].side == p.side && part(&sp[s]) == Some(part_g)) {
            taken[s] = true;
            source[g] = Some((s, How::Synonym));
        }
    }
    // 3. By position: under the nearest mapped ancestor, the nearest source joint below its source, both skeletons
    // brought to one height and their first mapped joints laid on each other.
    let gw: Vec<V3> = game.world().iter().map(|m| [m[12], m[13], m[14]]).collect();
    let height = |ps: &[V3]| {
        let (lo, hi) = ps.iter().fold((f32::MAX, f32::MIN), |(lo, hi), p| (lo.min(p[1]), hi.max(p[1])));
        (hi - lo).max(1e-6)
    };
    let game_height = height(&gw);
    // The scale from the joints already mapped, which both skeletons agree on: a joint only one of them has (a centre
    // below the hips, hair, a tail) would skew whole heights. Whole heights when the mapped ones stand at one height.
    let pairs: Vec<(usize, usize)> = source.iter().enumerate().filter_map(|(g, s)| s.as_ref().map(|(s, _)| (g, *s))).collect();
    let mapped_game = height(&pairs.iter().map(|&(g, _)| gw[g]).collect::<Vec<_>>());
    let mapped_source = height(&pairs.iter().map(|&(_, s)| source_world[s]).collect::<Vec<_>>());
    let scale = if pairs.len() >= 2 && mapped_game > 0.1 * game_height && mapped_source > 0.1 * height(source_world) {
        mapped_game / mapped_source
    } else {
        game_height / height(source_world)
    };
    let anchor = source.iter().enumerate().find_map(|(g, s)| s.as_ref().map(|(s, _)| (g, *s)));
    let offset = anchor.map_or([0.0; 3], |(g, s)| sub(gw[g], source_world[s].map(|v| v * scale)));
    let placed = |s: usize| -> V3 {
        let p = source_world[s].map(|v| v * scale);
        [p[0] + offset[0], p[1] + offset[1], p[2] + offset[2]]
    };
    let below = |s: usize, ancestor: usize| {
        let mut at = source_parents.get(s).copied().flatten();
        let mut steps = 0;
        while let Some(p) = at {
            if p == ancestor {
                return true;
            }
            steps += 1;
            if steps > source_parents.len() {
                return false;
            }
            at = source_parents.get(p).copied().flatten();
        }
        false
    };
    for g in 0..game.names.len() {
        if source[g].is_some() || fixed[g] {
            continue;
        }
        let mut up = game.parents[g];
        let mut steps = 0;
        while let Some(a) = up {
            if source[a].is_some() || steps > game.names.len() {
                break;
            }
            up = game.parents[a];
            steps += 1;
        }
        let Some(ancestor) = up.and_then(|a| source[a].as_ref().map(|(s, _)| *s)) else { continue };
        let best = (0..source_names.len())
            .filter(|&s| !taken[s] && below(s, ancestor))
            .map(|s| (s, length(sub(placed(s), gw[g]))))
            .min_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((s, distance)) = best {
            let score = (1.0 - distance / (0.25 * game_height)).clamp(0.0, 1.0);
            if score >= GUESS_SCORE {
                taken[s] = true;
                source[g] = Some((s, How::Position(score)));
            }
        }
    }
    Mapped { source }
}

// ------------------------------------------------------------------ the glb

struct Glb {
    json: Value,
    bin: Vec<u8>,
}

fn read_glb(bytes: &[u8]) -> Result<Glb> {
    let word = |at: usize| bytes.get(at..at + 4).map(|b| u32::from_le_bytes(b.try_into().expect("4")) as usize);
    if bytes.get(..4) != Some(b"glTF") || word(4) != Some(2) {
        bail!(say!("LumaLink.Character.NotGlb"));
    }
    let json_len = word(12).context(say!("LumaLink.Character.NotGlb"))?;
    if word(16) != Some(0x4E4F_534A) || bytes.len() < 20 + json_len {
        bail!(say!("LumaLink.Character.NotGlb"));
    }
    let json: Value = serde_json::from_slice(&bytes[20..20 + json_len]).map_err(|_| anyhow::anyhow!(say!("LumaLink.Character.NotGlb")))?;
    let at = 20 + json_len;
    let bin = match (word(at), word(at + 4)) {
        (Some(len), Some(0x004E_4942)) if bytes.len() >= at + 8 + len => bytes[at + 8..at + 8 + len].to_vec(),
        _ => Vec::new(),
    };
    if json["buffers"].as_array().is_some_and(|b| b.iter().any(|x| x.get("uri").is_some())) {
        bail!(say!("LumaLink.Character.OutsideBuffer"));
    }
    Ok(Glb { json, bin })
}

fn write_glb(json: &Value, bin: &[u8]) -> Vec<u8> {
    let mut text = serde_json::to_vec(json).expect("JSON");
    while text.len() % 4 != 0 {
        text.push(b' ');
    }
    let mut bin = bin.to_vec();
    while bin.len() % 4 != 0 {
        bin.push(0);
    }
    let total = 12 + 8 + text.len() + 8 + bin.len();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(text.len() as u32).to_le_bytes());
    out.extend_from_slice(&0x4E4F_534Au32.to_le_bytes());
    out.extend_from_slice(&text);
    out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    out.extend_from_slice(&0x004E_4942u32.to_le_bytes());
    out.extend_from_slice(&bin);
    out
}

/// Where an accessor's elements are in the BIN: (offset of element 0, stride, count, component type, components).
fn accessor(json: &Value, index: usize) -> Result<(usize, usize, usize, u64, usize)> {
    let bad = || anyhow::anyhow!(say!("LumaLink.Character.BadAccessor", index = index));
    let a = &json["accessors"][index];
    if a.get("sparse").is_some() {
        return Err(bad());
    }
    let view = &json["bufferViews"][a["bufferView"].as_u64().ok_or_else(bad)? as usize];
    let components = match a["type"].as_str().ok_or_else(bad)? {
        "SCALAR" => 1,
        "VEC2" => 2,
        "VEC3" => 3,
        "VEC4" => 4,
        "MAT4" => 16,
        _ => return Err(bad()),
    };
    let ty = a["componentType"].as_u64().ok_or_else(bad)?;
    let size = match ty {
        5120 | 5121 => 1,
        5122 | 5123 => 2,
        5125 | 5126 => 4,
        _ => return Err(bad()),
    };
    let start = view["byteOffset"].as_u64().unwrap_or(0) as usize + a["byteOffset"].as_u64().unwrap_or(0) as usize;
    let stride = view["byteStride"].as_u64().map_or(size * components, |s| s as usize);
    let count = a["count"].as_u64().ok_or_else(bad)? as usize;
    Ok((start, stride, count, ty, components))
}

/// An accessor's values as floats (normalised integers turned to 0..1).
fn read_floats(glb: &Glb, index: usize) -> Result<Vec<f32>> {
    let (start, stride, count, ty, n) = accessor(&glb.json, index)?;
    let normalized = glb.json["accessors"][index]["normalized"].as_bool().unwrap_or(false);
    let mut out = Vec::with_capacity(count * n);
    for e in 0..count {
        for c in 0..n {
            let at = start + e * stride;
            let v = match ty {
                5126 => glb.bin.get(at + c * 4..at + c * 4 + 4).map(|b| f32::from_le_bytes(b.try_into().expect("4"))),
                5121 => glb.bin.get(at + c).map(|b| if normalized { f32::from(*b) / 255.0 } else { f32::from(*b) }),
                5123 => glb.bin.get(at + c * 2..at + c * 2 + 2).map(|b| {
                    let v = f32::from(u16::from_le_bytes(b.try_into().expect("2")));
                    if normalized { v / 65535.0 } else { v }
                }),
                5125 => glb.bin.get(at + c * 4..at + c * 4 + 4).map(|b| u32::from_le_bytes(b.try_into().expect("4")) as f32),
                _ => None,
            };
            out.push(v.ok_or_else(|| anyhow::anyhow!(say!("LumaLink.Character.BadAccessor", index = index)))?);
        }
    }
    Ok(out)
}

/// Writes floats into a float accessor in place.
fn write_floats(glb: &mut Glb, index: usize, values: &[f32]) -> Result<()> {
    let (start, stride, count, ty, n) = accessor(&glb.json, index)?;
    if ty != 5126 || values.len() != count * n {
        bail!(say!("LumaLink.Character.BadAccessor", index = index));
    }
    for e in 0..count {
        for c in 0..n {
            let at = start + e * stride + c * 4;
            glb.bin.get_mut(at..at + 4).context(say!("LumaLink.Character.BadAccessor", index = index))?.copy_from_slice(&values[e * n + c].to_le_bytes());
        }
    }
    Ok(())
}

/// Writes joint numbers into a JOINTS_0 accessor in place (u8 or u16).
fn write_joints(glb: &mut Glb, index: usize, values: &[u32]) -> Result<()> {
    let (start, stride, count, ty, n) = accessor(&glb.json, index)?;
    if values.len() != count * n {
        bail!(say!("LumaLink.Character.BadAccessor", index = index));
    }
    for e in 0..count {
        for c in 0..n {
            let v = values[e * n + c];
            match ty {
                5121 => {
                    let b = u8::try_from(v).map_err(|_| anyhow::anyhow!(say!("LumaLink.Character.TooManyJoints", count = v + 1)))?;
                    *glb.bin.get_mut(start + e * stride + c).context(say!("LumaLink.Character.BadAccessor", index = index))? = b;
                }
                5123 => {
                    let b = u16::try_from(v).map_err(|_| anyhow::anyhow!(say!("LumaLink.Character.TooManyJoints", count = v + 1)))?;
                    let at = start + e * stride + c * 2;
                    glb.bin.get_mut(at..at + 2).context(say!("LumaLink.Character.BadAccessor", index = index))?.copy_from_slice(&b.to_le_bytes());
                }
                _ => bail!(say!("LumaLink.Character.BadAccessor", index = index)),
            }
        }
    }
    Ok(())
}

/// A node's transform from its parent (matrix, or TRS).
fn node_local(node: &Value) -> Mat {
    if let Some(m) = node["matrix"].as_array().filter(|m| m.len() == 16) {
        let mut out = IDENTITY;
        for (i, v) in m.iter().enumerate() {
            out[i] = v.as_f64().unwrap_or(0.0) as f32;
        }
        return out;
    }
    let f = |key: &str, n: usize, default: &[f32]| -> Vec<f32> {
        node[key].as_array().filter(|a| a.len() == n).map_or_else(|| default.to_vec(), |a| a.iter().map(|v| v.as_f64().unwrap_or(0.0) as f32).collect())
    };
    let (t, r, s) = (f("translation", 3, &[0.0; 3]), f("rotation", 4, &[0.0, 0.0, 0.0, 1.0]), f("scale", 3, &[1.0; 3]));
    trs([t[0], t[1], t[2]], [r[0], r[1], r[2], r[3]], [s[0], s[1], s[2]])
}

// ------------------------------------------------------------------ the whole step

/// What `add character` made and found.
#[derive(Clone, Debug, PartialEq)]
pub struct Report {
    pub game_model: String,
    /// Each game joint: its name and its source joint with how, or None (missing).
    pub joints: Vec<(String, Option<(String, How)>)>,
    /// Source joints the game lacks, kept as extra joints.
    pub extra: Vec<String>,
    /// Bones turned more than [`BENT_DEGREES`] to meet the game's rest pose: the game joint and the degrees.
    pub bent: Vec<(String, f32)>,
    /// The source's size times this is the game character's.
    pub scale: f32,
    pub dropped_animations: usize,
    pub vertices: usize,
}

/// The source .glb made into the game model's stand-in: the new .glb and the report.
pub fn rebind(source: &[u8], game: &GameSkeleton, choices: &[(String, String)]) -> Result<(Vec<u8>, Report)> {
    let mut glb = read_glb(source)?;
    let skins = glb.json["skins"].as_array().cloned().unwrap_or_default();
    let skin = skins.first().ok_or_else(|| anyhow::anyhow!(say!("LumaLink.Character.NoSkin")))?.clone();
    let joint_nodes: Vec<usize> = skin["joints"].as_array().map(|a| a.iter().filter_map(|v| v.as_u64().map(|n| n as usize)).collect()).unwrap_or_default();
    if joint_nodes.is_empty() {
        bail!(say!("LumaLink.Character.NoSkin"));
    }
    let nodes = glb.json["nodes"].as_array().cloned().unwrap_or_default();
    // Each node's parent, and its rest place in the model.
    let mut node_parent = vec![None; nodes.len()];
    for (i, n) in nodes.iter().enumerate() {
        for c in n["children"].as_array().into_iter().flatten().filter_map(Value::as_u64) {
            if let Some(slot) = node_parent.get_mut(c as usize) {
                *slot = Some(i);
            }
        }
    }
    let locals: Vec<Mat> = nodes.iter().map(node_local).collect();
    let mut worlds: Vec<Option<Mat>> = vec![None; nodes.len()];
    for i in 0..nodes.len() {
        world_of(i, &node_parent, &locals, &mut worlds, 0);
    }
    let node_world: Vec<Mat> = worlds.into_iter().map(|m| m.unwrap_or(IDENTITY)).collect();
    // The source joints' bind places: the inverse of their inverse bind matrices, else their rest nodes.
    let binds: Vec<Mat> = match skin["inverseBindMatrices"].as_u64() {
        Some(acc) => {
            let v = read_floats(&glb, acc as usize)?;
            if v.len() != joint_nodes.len() * 16 {
                bail!(say!("LumaLink.Character.BadAccessor", index = acc));
            }
            v.chunks_exact(16).map(|c| inverse(&c.try_into().expect("16"))).collect()
        }
        None => joint_nodes.iter().map(|&n| node_world[n]).collect(),
    };
    let names: Vec<String> = joint_nodes.iter().enumerate().map(|(j, &n)| nodes[n]["name"].as_str().map_or_else(|| format!("joint{j}"), str::to_string)).collect();
    let parents: Vec<Option<usize>> = joint_nodes
        .iter()
        .map(|&n| {
            let mut up = node_parent[n];
            while let Some(p) = up {
                if let Some(j) = joint_nodes.iter().position(|&x| x == p) {
                    return Some(j);
                }
                up = node_parent[p];
            }
            None
        })
        .collect();
    let source_world: Vec<V3> = binds.iter().map(|m| [m[12], m[13], m[14]]).collect();
    let given = given_joints(choices, game, &names)?;
    let mapped = map_joints(game, &names, &parents, &source_world, &given);
    let count = mapped.source.iter().filter(|s| s.is_some()).count();
    if count < FEWEST_MAPPED {
        bail!(say!("LumaLink.Character.TooFew", count = count, least = FEWEST_MAPPED, model = game.path));
    }
    // Rescale: the skeletons' heights.
    let game_world = game.world();
    let gpos: Vec<V3> = game_world.iter().map(|m| [m[12], m[13], m[14]]).collect();
    let height = |ps: &[V3]| {
        let (lo, hi) = ps.iter().fold((f32::MAX, f32::MIN), |(lo, hi), p| (lo.min(p[1]), hi.max(p[1])));
        (hi - lo).max(1e-6)
    };
    // Game units per source metre.
    let sigma = height(&gpos) / height(&source_world);
    let spos: Vec<V3> = source_world.iter().map(|p| p.map(|v| v * sigma)).collect();
    // Each source joint's game joint, and each game joint's turn onto the game's bone.
    let mut game_of_source: Vec<Option<usize>> = vec![None; names.len()];
    for (g, s) in mapped.source.iter().enumerate() {
        if let Some((s, _)) = s {
            game_of_source[*s] = Some(g);
        }
    }
    let g_count = game.names.len();
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); g_count];
    for (g, p) in game.parents.iter().enumerate() {
        if let Some(p) = p {
            if *p < g_count {
                children[*p].push(g);
            }
        }
    }
    let subtree = |g: usize| -> usize {
        let mut n = 0;
        let mut stack = vec![g];
        while let Some(x) = stack.pop() {
            n += 1;
            if n > g_count {
                break;
            }
            stack.extend(children[x].iter().copied());
        }
        n
    };
    // The nearest mapped game joints below each one, the one with the most below it first.
    let mapped_below = |g: usize| -> Option<usize> {
        let mut best: Option<(usize, usize)> = None;
        for &c in &children[g] {
            let mut stack = vec![c];
            let mut seen = 0;
            while let Some(x) = stack.pop() {
                seen += 1;
                if seen > g_count {
                    break;
                }
                if mapped.source[x].is_some() {
                    let size = subtree(x);
                    if best.is_none_or(|(_, s)| size > s) {
                        best = Some((x, size));
                    }
                } else {
                    stack.extend(children[x].iter().copied());
                }
            }
        }
        best.map(|(x, _)| x)
    };
    // Game joints in parent-first order.
    let mut order: Vec<usize> = Vec::with_capacity(g_count);
    let mut stack: Vec<usize> = (0..g_count).filter(|&g| game.parents[g].is_none_or(|p| p >= g_count)).rev().collect();
    while let Some(g) = stack.pop() {
        if order.len() >= g_count {
            break;
        }
        order.push(g);
        stack.extend(children[g].iter().rev().copied());
    }
    let mut turn: Vec<Option<Quat>> = vec![None; g_count];
    let mut bent = Vec::new();
    for &g in &order {
        let inherited = game.parents[g].and_then(|p| turn.get(p).copied().flatten()).unwrap_or([0.0, 0.0, 0.0, 1.0]);
        let q = match (&mapped.source[g], mapped_below(g)) {
            (Some((s, _)), Some(c)) => {
                let sc = mapped.source[c].as_ref().map(|(s, _)| *s).expect("mapped");
                let q = arc(sub(spos[sc], spos[*s]), sub(gpos[c], gpos[g]));
                let degrees = quat_degrees(q);
                if degrees > BENT_DEGREES {
                    bent.push((game.names[g].clone(), degrees));
                }
                q
            }
            _ => inherited,
        };
        turn[g] = Some(q);
    }
    // The map of each source joint's vertices: source metres to output metres.
    let anchor = (0..g_count).find_map(|g| mapped.source[g].as_ref().map(|(s, _)| (g, *s))).expect("mapped");
    let map_of = |g: usize, s: usize| -> Mat {
        let q = turn[g].unwrap_or([0.0, 0.0, 0.0, 1.0]);
        let r = quat_mat(q);
        let k = sigma / UNITS_PER_METRE;
        let mut m = r;
        for c in 0..3 {
            for row in 0..3 {
                m[c * 4 + row] *= k;
            }
        }
        let moved = vector(&r, spos[s]);
        for i in 0..3 {
            m[12 + i] = (gpos[g][i] - moved[i]) / UNITS_PER_METRE;
        }
        m
    };
    // Each source joint: its game joint (mapped) or its nearest mapped ancestor's (an extra joint).
    let mut maps: Vec<Mat> = Vec::with_capacity(names.len());
    let mut owner: Vec<usize> = Vec::with_capacity(names.len());
    for s in 0..names.len() {
        let mut at = Some(s);
        let mut found = None;
        let mut steps = 0;
        while let Some(x) = at {
            if let Some(g) = game_of_source[x] {
                found = Some((g, x));
                break;
            }
            steps += 1;
            if steps > names.len() {
                break;
            }
            at = parents[x];
        }
        let (g, x) = found.unwrap_or(anchor);
        maps.push(map_of(g, x));
        owner.push(g);
    }
    // The output skeleton: the game's joints (its rest pose, metres), then the extra joints.
    let game_metres: Vec<Mat> = game_world
        .iter()
        .map(|m| {
            let mut m = *m;
            m[12] /= UNITS_PER_METRE;
            m[13] /= UNITS_PER_METRE;
            m[14] /= UNITS_PER_METRE;
            m
        })
        .collect();
    let extra: Vec<usize> = (0..names.len()).filter(|&s| game_of_source[s].is_none()).collect();
    let mut out_world: Vec<Mat> = game_metres.clone();
    let mut out_index: Vec<u32> = vec![0; names.len()];
    for (s, slot) in out_index.iter_mut().enumerate() {
        if let Some(g) = game_of_source[s] {
            *slot = g as u32;
        }
    }
    for (k, &s) in extra.iter().enumerate() {
        out_index[s] = (g_count + k) as u32;
        let p = point(&maps[s], source_world[s]);
        let mut w = IDENTITY;
        w[12] = p[0];
        w[13] = p[1];
        w[14] = p[2];
        out_world.push(w);
    }
    // The mesh: every skinned primitive's vertices moved by their weights, normals turned, joints renumbered.
    let mut done = std::collections::BTreeSet::new();
    let mut vertices = 0usize;
    let meshes = glb.json["meshes"].as_array().cloned().unwrap_or_default();
    for mesh in &meshes {
        for prim in mesh["primitives"].as_array().into_iter().flatten() {
            let attrs = &prim["attributes"];
            let (Some(pos), Some(jo), Some(we)) = (attrs["POSITION"].as_u64(), attrs["JOINTS_0"].as_u64(), attrs["WEIGHTS_0"].as_u64()) else { continue };
            if !done.insert(pos) {
                continue;
            }
            let positions = read_floats(&glb, pos as usize)?;
            let joints = read_floats(&glb, jo as usize)?;
            let weights = read_floats(&glb, we as usize)?;
            let n = positions.len() / 3;
            if joints.len() != n * 4 || weights.len() != n * 4 {
                bail!(say!("LumaLink.Character.BadAccessor", index = jo));
            }
            let normals = match attrs["NORMAL"].as_u64() {
                Some(nr) => Some((nr, read_floats(&glb, nr as usize)?)),
                None => None,
            };
            let mut new_pos = vec![0.0f32; n * 3];
            let mut new_nrm = normals.as_ref().map(|(_, v)| vec![0.0f32; v.len()]);
            let mut new_joints = vec![0u32; n * 4];
            for v in 0..n {
                let p = [positions[v * 3], positions[v * 3 + 1], positions[v * 3 + 2]];
                let total: f32 = weights[v * 4..v * 4 + 4].iter().sum();
                let mut acc = [0.0f32; 3];
                let mut nacc = [0.0f32; 3];
                for k in 0..4 {
                    let j = joints[v * 4 + k] as usize;
                    let w = if total > 1e-6 { weights[v * 4 + k] / total } else if k == 0 { 1.0 } else { 0.0 };
                    let j = j.min(names.len() - 1);
                    new_joints[v * 4 + k] = out_index[j];
                    if w == 0.0 {
                        continue;
                    }
                    let q = point(&maps[j], p);
                    for i in 0..3 {
                        acc[i] += w * q[i];
                    }
                    if let Some((_, nv)) = &normals {
                        let t = vector(&quat_mat(turn[owner[j]].unwrap_or([0.0, 0.0, 0.0, 1.0])), [nv[v * 3], nv[v * 3 + 1], nv[v * 3 + 2]]);
                        for i in 0..3 {
                            nacc[i] += w * t[i];
                        }
                    }
                }
                new_pos[v * 3..v * 3 + 3].copy_from_slice(&acc);
                if let Some(out) = new_nrm.as_mut() {
                    let l = length(nacc).max(1e-9);
                    out[v * 3..v * 3 + 3].copy_from_slice(&nacc.map(|x| x / l));
                }
            }
            write_floats(&mut glb, pos as usize, &new_pos)?;
            if let (Some((nr, _)), Some(out)) = (normals, new_nrm) {
                write_floats(&mut glb, nr as usize, &out)?;
            }
            write_joints(&mut glb, jo as usize, &new_joints)?;
            // The bounds glTF asks of a POSITION accessor.
            let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
            for v in new_pos.chunks_exact(3) {
                for i in 0..3 {
                    lo[i] = lo[i].min(v[i]);
                    hi[i] = hi[i].max(v[i]);
                }
            }
            glb.json["accessors"][pos as usize]["min"] = json!(lo);
            glb.json["accessors"][pos as usize]["max"] = json!(hi);
            vertices += n;
        }
    }
    // The nodes: the game's joints and the extra joints are new nodes; the source's joints leave the scene; a node
    // that hung on a source joint hangs on its new joint, where its joint's map puts it.
    let first = nodes.len();
    let mut new_nodes = nodes.clone();
    for n in new_nodes.iter_mut() {
        if let Some(children) = n.get_mut("children").and_then(Value::as_array_mut) {
            children.retain(|c| c.as_u64().is_none_or(|c| !joint_nodes.contains(&(c as usize))));
        }
    }
    let out_count = g_count + extra.len();
    let parent_of_out = |o: usize| -> Option<usize> {
        if o < g_count {
            game.parents[o].filter(|p| *p < g_count)
        } else {
            let s = extra[o - g_count];
            // Its source parent's output joint; a source root hangs on the game joint its vertices follow.
            Some(parents[s].map_or(owner[s], |p| out_index[p] as usize))
        }
    };
    let mut out_children: Vec<Vec<usize>> = vec![Vec::new(); out_count];
    let mut roots = Vec::new();
    for o in 0..out_count {
        match parent_of_out(o) {
            Some(p) if p != o => out_children[p].push(o),
            _ => roots.push(o),
        }
    }
    for o in 0..out_count {
        let name = if o < g_count { game.names[o].clone() } else { names[extra[o - g_count]].clone() };
        let mut node = serde_json::Map::new();
        node.insert("name".into(), json!(name));
        if o < g_count {
            let l = &game.local[o];
            // The game's rest pose as it is, in metres: the cook reads it back as the game's angles.
            let mut m = *l;
            m[12] /= UNITS_PER_METRE;
            m[13] /= UNITS_PER_METRE;
            m[14] /= UNITS_PER_METRE;
            node.insert("matrix".into(), json!(m));
        } else {
            let parent_world = parent_of_out(o).map_or(IDENTITY, |p| out_world[p]);
            node.insert("matrix".into(), json!(mul(&inverse(&parent_world), &out_world[o])));
        }
        if !out_children[o].is_empty() {
            node.insert("children".into(), json!(out_children[o].iter().map(|c| first + c).collect::<Vec<_>>()));
        }
        new_nodes.push(Value::Object(node));
    }
    // Nodes that hung on a source joint (a hat, a held thing): onto the new joint.
    for (j, &jn) in joint_nodes.iter().enumerate() {
        for c in nodes[jn]["children"].as_array().into_iter().flatten().filter_map(Value::as_u64).map(|c| c as usize) {
            if joint_nodes.contains(&c) {
                continue;
            }
            let o = out_index[j] as usize;
            let world = mul(&maps[j], &node_world[c]);
            let local = mul(&inverse(&out_world[o]), &world);
            if let Some(node) = new_nodes.get_mut(c).and_then(Value::as_object_mut) {
                for key in ["translation", "rotation", "scale"] {
                    node.remove(key);
                }
                node.insert("matrix".into(), json!(local));
            }
            if let Some(children) = new_nodes[first + o].as_object_mut().map(|n| n.entry("children").or_insert_with(|| json!([]))) {
                if let Some(list) = children.as_array_mut() {
                    list.push(json!(c));
                }
            }
        }
    }
    // The scene: the new skeleton's roots, and the old roots that are no source joints.
    let scene_index = glb.json["scene"].as_u64().unwrap_or(0) as usize;
    let old_roots: Vec<usize> = glb.json["scenes"][scene_index]["nodes"].as_array().into_iter().flatten().filter_map(Value::as_u64).map(|n| n as usize).filter(|n| !joint_nodes.contains(n)).collect();
    let mut scene_nodes: Vec<usize> = roots.iter().map(|r| first + r).collect();
    scene_nodes.extend(old_roots.into_iter().filter(|n| !joint_nodes.iter().any(|j| *j == *n)));
    // The skin: the new joints, their inverse bind matrices appended to the BIN.
    while glb.bin.len() % 4 != 0 {
        glb.bin.push(0);
    }
    let ibm_at = glb.bin.len();
    for w in &out_world {
        for v in inverse(w) {
            glb.bin.extend_from_slice(&v.to_le_bytes());
        }
    }
    let views = glb.json["bufferViews"].as_array().map_or(0, Vec::len);
    let accessors = glb.json["accessors"].as_array().map_or(0, Vec::len);
    if glb.json["bufferViews"].as_array().is_none() {
        glb.json["bufferViews"] = json!([]);
    }
    if glb.json["accessors"].as_array().is_none() {
        glb.json["accessors"] = json!([]);
    }
    glb.json["bufferViews"].as_array_mut().expect("list").push(json!({ "buffer": 0, "byteOffset": ibm_at, "byteLength": out_world.len() * 64 }));
    glb.json["accessors"].as_array_mut().expect("list").push(json!({ "bufferView": views, "componentType": 5126, "count": out_world.len(), "type": "MAT4" }));
    let joints_list: Vec<usize> = (0..out_count).map(|o| first + o).collect();
    let skeleton_root = roots.first().map(|r| first + r);
    glb.json["skins"] = json!([{ "joints": joints_list, "inverseBindMatrices": accessors, "skeleton": skeleton_root, "name": game.path }]);
    glb.json["nodes"] = Value::Array(new_nodes);
    if glb.json["scenes"].as_array().is_none_or(Vec::is_empty) {
        glb.json["scenes"] = json!([{ "nodes": [] }]);
        glb.json["scene"] = json!(0);
    }
    glb.json["scenes"][scene_index]["nodes"] = json!(scene_nodes);
    let dropped_animations = glb.json["animations"].as_array().map_or(0, Vec::len);
    if let Some(o) = glb.json.as_object_mut() {
        o.remove("animations");
    }
    if glb.json["buffers"].as_array().is_none_or(Vec::is_empty) {
        glb.json["buffers"] = json!([{}]);
    }
    glb.json["buffers"][0]["byteLength"] = json!(glb.bin.len().div_ceil(4) * 4);
    let joints_report = (0..g_count)
        .map(|g| (game.names[g].clone(), mapped.source[g].as_ref().map(|(s, how)| (names[*s].clone(), how.clone()))))
        .collect();
    let report = Report {
        game_model: game.path.clone(),
        joints: joints_report,
        extra: extra.iter().map(|&s| names[s].clone()).collect(),
        bent,
        scale: sigma / UNITS_PER_METRE,
        dropped_animations,
        vertices,
    };
    Ok((write_glb(&glb.json, &glb.bin), report))
}

/// The report as lines in the chosen language.
pub fn report_lines(r: &Report) -> Vec<String> {
    let mut out = vec![say!("LumaLink.Character.Report", model = r.game_model, scale = format!("{:.3}", r.scale), vertices = r.vertices)];
    for (game, found) in &r.joints {
        out.push(match found {
            Some((source, How::Given)) => say!("LumaLink.Character.Given", joint = game, source = source),
            Some((source, How::Name)) => say!("LumaLink.Character.ByName", joint = game, source = source),
            Some((source, How::Synonym)) => say!("LumaLink.Character.BySynonym", joint = game, source = source),
            Some((source, How::Position(score))) => say!("LumaLink.Character.Guessed", joint = game, source = source, score = format!("{score:.2}")),
            None => say!("LumaLink.Character.Missing", joint = game),
        });
    }
    for e in &r.extra {
        out.push(say!("LumaLink.Character.Extra", joint = e));
    }
    for (joint, degrees) in &r.bent {
        out.push(say!("LumaLink.Character.Bent", joint = joint, degrees = format!("{degrees:.0}"), most = BENT_DEGREES));
    }
    if r.dropped_animations > 0 {
        out.push(say!("LumaLink.Character.Animations", count = r.dropped_animations));
    }
    out
}

/// Where the .glb goes in a mod: `layers/base/files/<the game model's path>.glb`, so it stands in for exactly that
/// file (models 's `<Name>.bdl.glb` form).
pub fn place_in_mod(folder: &Path, game_path: &str) -> std::path::PathBuf {
    let mut out = folder.join("layers").join("base").join("files");
    for part in game_path.split('/') {
        out.push(part);
    }
    let mut name = out.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(".glb");
    out.set_file_name(name);
    out
}

/// Where `add character --as <at>` writes: `layers/base/assets/<at>.glb`, a new model of the mod (docs/modding/content/models.md),
/// `.glb` added when `at` lacks it. None for a path that is not a mod path: empty, absolute, with a drive, `.` or
/// `..` parts, a backslash, or another ending.
pub fn asset_in_mod(folder: &Path, at: &str) -> Option<std::path::PathBuf> {
    let at = at.strip_suffix(".glb").unwrap_or(at);
    let parts: Vec<&str> = at.split('/').collect();
    if at.is_empty() || at.contains('\\') || at.contains(':') || at.contains('.')
        || parts.iter().any(|p| p.is_empty() || *p == "." || *p == "..")
    {
        return None;
    }
    let mut out = folder.join("layers").join("base").join("assets");
    for part in &parts {
        out.push(part);
    }
    let mut name = out.file_name()?.to_os_string();
    name.push(".glb");
    out.set_file_name(name);
    Some(out)
}

/// The game's separate part models a player character is drawn with (its hands and face, swapped by the game for
/// gestures and expressions): a new character has its own, so `add character` hides these.
pub fn part_models(character: &str) -> &'static [&'static str] {
    match character.to_ascii_lowercase().as_str() {
        "mario" => &["MarioHandL", "MarioHandR", "MarioFace"],
        "luigi" => &["LuigiHandL", "LuigiHandR", "LuigiFace"],
        _ => &[],
    }
}

/// Where a hidden game model's entry goes: `layers/base/files/<the game model's path>.model.json`, holding
/// `{"hide": true}` (the cook writes a model with the game's joints and no meshes).
pub fn hide_in_mod(folder: &Path, game_path: &str) -> std::path::PathBuf {
    let glb = place_in_mod(folder, game_path);
    glb.with_extension("model.json")
}

/// The text of a hidden model's entry.
pub const HIDE_ENTRY: &str = "{ \"hide\": true }\n";

/// The names joints by their parts, for the tests and for a modder's question.
pub fn parts_of(names: &[&str]) -> BTreeMap<String, Option<Part>> {
    names.iter().map(|n| (n.to_string(), part(&name_parts(n)))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tool_s_name_for_a_forearm_is_one_part() {
        for name in ["mixamorig:LeftForeArm", "J_Bip_L_LowerArm", "DEF-forearm.L", "lowerarm_l", "ArmL2", "Left Elbow"] {
            let p = name_parts(name);
            assert_eq!((part(&p), p.side), (Some(Part::LowerArm), Side::Left), "{name}: {p:?}");
        }
        for name in ["mixamorig:RightArm", "J_Bip_R_UpperArm", "upper_arm.R", "upperarm_r", "ArmR1"] {
            let p = name_parts(name);
            assert_eq!((part(&p), p.side), (Some(Part::UpperArm), Side::Right), "{name}: {p:?}");
        }
        assert_eq!(part(&name_parts("mixamorig:LeftLeg")), Some(Part::LowerLeg));
        assert_eq!(part(&name_parts("LegL1")), Some(Part::UpperLeg));
        assert_eq!(part(&name_parts("J_Bip_C_Hips")), Some(Part::Pelvis));
        assert_eq!(part(&name_parts("Spine2")), Some(Part::Chest));
        assert_eq!(part(&name_parts("Hair_01")), None);
        assert_eq!(part(&name_parts("CapPosition")), Some(Part::Hat));
        assert_eq!(part(&name_parts("Cap")), Some(Part::Hat));
        assert_eq!(part(&name_parts("J_Bip_C_Helmet")), Some(Part::Hat));
    }

    #[test]
    fn a_cap_goes_to_the_game_s_cap_and_not_to_a_joint_near_it() {
        // The game's head with its cap locator and the cap; a source (SMO Mario's) whose mouth lies nearer the
        // locator than its cap does: the cap is mapped by its part, the mouth stays the source's own.
        let game = skeleton(&["Head", "CapPosition", "Cap0"], &[None, Some(0), Some(1)], &[[0.0, 150.0, 0.0], [0.0, 160.0, 0.0], [0.0, 165.0, 0.0]]);
        let names: Vec<String> = ["Head", "MouthRoot", "Cap"].iter().map(|s| s.to_string()).collect();
        let places = [[0.0, 1.5, 0.0], [0.0, 1.6, 0.0], [0.0, 1.75, 0.0]];
        let m = map_joints(&game, &names, &[None, Some(0), Some(0)], &places, &Vec::new());
        assert_eq!(m.source[1], Some((2, How::Synonym)));
        assert!(m.source.iter().all(|s| s.as_ref().map_or(true, |(s, _)| *s != 1)), "{:?}", m.source);
        // The maker's choices win: Cap0 given the cap, CapPosition given none; a name either side lacks is refused.
        let own = |pairs: &[(&str, &str)]| pairs.iter().map(|(g, s)| (g.to_string(), s.to_string())).collect::<Vec<_>>();
        let given = given_joints(&own(&[("Cap0", "Cap"), ("CapPosition", "")]), &game, &names).unwrap();
        let m = map_joints(&game, &names, &[None, Some(0), Some(0)], &places, &given);
        assert_eq!((m.source[1].clone(), m.source[2].clone()), (None, Some((2, How::Given))));
        assert!(given_joints(&own(&[("Hat", "Cap")]), &game, &names).is_err());
        assert!(given_joints(&own(&[("Cap0", "Hat")]), &game, &names).is_err());
        assert!(given_joints(&own(&[("Cap0", "Cap"), ("CapPosition", "Cap")]), &game, &names).is_err());
    }

    fn skeleton(names: &[&str], parents: &[Option<usize>], places: &[V3]) -> GameSkeleton {
        let world: Vec<V3> = places.to_vec();
        let local = (0..names.len())
            .map(|j| {
                let p = parents[j].map_or([0.0; 3], |p| world[p]);
                trs(sub(world[j], p), [0.0, 0.0, 0.0, 1.0], [1.0; 3])
            })
            .collect();
        GameSkeleton { path: "ObjectData/Mario.arc/Mario.bdl".into(), names: names.iter().map(|s| s.to_string()).collect(), parents: parents.to_vec(), local }
    }

    #[test]
    fn as_asset_paths_are_the_mods_assets() {
        let f = Path::new("m");
        let assets = f.join("layers").join("base").join("assets");
        assert_eq!(asset_in_mod(f, "GoldMario"), Some(assets.join("GoldMario.glb")));
        assert_eq!(asset_in_mod(f, "GoldMario.glb"), Some(assets.join("GoldMario.glb")));
        assert_eq!(asset_in_mod(f, "Figures/Gold_Mario-2"), Some(assets.join("Figures").join("Gold_Mario-2.glb")));
        for bad in ["", ".glb", "/Gold", "C:/Gold", "../Gold", "a/../b", "a//b", "a/", "a\\b", "Gold.gltf", "a.b/c"] {
            assert_eq!(asset_in_mod(f, bad), None, "{bad}");
        }
    }

    #[test]
    fn names_then_synonyms_then_places_map_the_joints() {
        // The game: Center, Hip, Spine1, Head, ArmL1, ArmL2, HandL; in units, 150 tall.
        let game = skeleton(
            &["Center", "Hip", "Spine1", "Head", "ArmL1", "ArmL2", "HandL"],
            &[None, Some(0), Some(1), Some(2), Some(2), Some(4), Some(5)],
            &[[0.0, 50.0, 0.0], [0.0, 60.0, 0.0], [0.0, 90.0, 0.0], [0.0, 150.0, 0.0], [20.0, 110.0, 0.0], [45.0, 110.0, 0.0], [70.0, 110.0, 0.0]],
        );
        // A Mixamo-like source in metres, 1.5 tall, its wrist named oddly.
        let names: Vec<String> = ["mixamorig:Hips", "mixamorig:Spine1", "mixamorig:Head", "mixamorig:LeftArm", "mixamorig:LeftForeArm", "Wrist_thing"].iter().map(|s| s.to_string()).collect();
        let parents = [None, Some(0), Some(1), Some(1), Some(3), Some(4)];
        let world = [[0.0, 0.6, 0.0], [0.0, 0.9, 0.0], [0.0, 1.5, 0.0], [0.2, 1.1, 0.0], [0.45, 1.1, 0.0], [0.7, 1.1, 0.0]];
        let m = map_joints(&game, &names, &parents, &world, &Vec::new());
        assert_eq!(m.source[2], Some((1, How::Name)));
        assert_eq!(m.source[1], Some((0, How::Synonym)));
        assert_eq!(m.source[4], Some((3, How::Synonym)));
        assert_eq!(m.source[5], Some((4, How::Synonym)));
        assert!(matches!(m.source[6], Some((5, How::Position(s))) if s > 0.9));
        assert_eq!(m.source[0], None);
    }

    #[test]
    fn the_turn_onto_a_bone_and_the_game_angles_read_as_written() {
        let q = arc([1.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        let v = vector(&quat_mat(q), [1.0, 0.0, 0.0]);
        assert!((v[1] - 1.0).abs() < 1e-5 && v[0].abs() < 1e-5);
        assert!((quat_degrees(q) - 90.0).abs() < 1e-3);
        // Z·Y·X: a quarter turn about Z alone turns X onto Y.
        let z = vector(&quat_mat(euler_zyx([0.0, 0.0, std::f32::consts::FRAC_PI_2])), [1.0, 0.0, 0.0]);
        assert!((z[1] - 1.0).abs() < 1e-5);
        let m = trs([1.0, 2.0, 3.0], euler_zyx([0.3, -0.2, 0.9]), [1.0; 3]);
        let back = mul(&m, &inverse(&m));
        for (a, b) in back.iter().zip(IDENTITY.iter()) {
            assert!((a - b).abs() < 1e-5);
        }
    }

    #[test]
    fn the_model_goes_at_the_game_model_s_place_in_the_base_layer() {
        let p = place_in_mod(Path::new("mod"), "ObjectData/Mario.arc/Mario.bdl");
        assert_eq!(p, Path::new("mod").join("layers").join("base").join("files").join("ObjectData").join("Mario.arc").join("Mario.bdl.glb"));
    }

    /// A one-triangle character with a three-joint chain, made into the game's skeleton: the joints are the game's,
    /// the vertices follow their joints, and the glb reads back.
    #[test]
    fn a_small_character_is_rebound_onto_the_game_s_skeleton() {
        let game = skeleton(&["Hip", "Spine1", "Head"], &[None, Some(0), Some(1)], &[[0.0, 50.0, 0.0], [0.0, 90.0, 0.0], [0.0, 150.0, 0.0]]);
        let positions: [f32; 9] = [0.0, 0.5, 0.0, 0.0, 0.9, 0.0, 0.0, 1.5, 0.0];
        let joints: [u8; 12] = [0, 0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0];
        let weights: [f32; 12] = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0];
        let mut bin = Vec::new();
        positions.iter().for_each(|v| bin.extend_from_slice(&v.to_le_bytes()));
        bin.extend_from_slice(&joints);
        weights.iter().for_each(|v| bin.extend_from_slice(&v.to_le_bytes()));
        let json = json!({
            "asset": { "version": "2.0" },
            "buffers": [{ "byteLength": bin.len() }],
            "bufferViews": [
                { "buffer": 0, "byteOffset": 0, "byteLength": 36 },
                { "buffer": 0, "byteOffset": 36, "byteLength": 12 },
                { "buffer": 0, "byteOffset": 48, "byteLength": 48 }
            ],
            "accessors": [
                { "bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3" },
                { "bufferView": 1, "componentType": 5121, "count": 3, "type": "VEC4" },
                { "bufferView": 2, "componentType": 5126, "count": 3, "type": "VEC4" }
            ],
            "meshes": [{ "primitives": [{ "attributes": { "POSITION": 0, "JOINTS_0": 1, "WEIGHTS_0": 2 } }] }],
            "nodes": [
                { "name": "Hips", "translation": [0.0, 0.5, 0.0], "children": [1] },
                { "name": "Spine1", "translation": [0.0, 0.4, 0.0], "children": [2] },
                { "name": "Head", "translation": [0.0, 0.6, 0.0] },
                { "name": "Body", "mesh": 0, "skin": 0 }
            ],
            "skins": [{ "joints": [0, 1, 2] }],
            "scenes": [{ "nodes": [0, 3] }],
            "scene": 0
        });
        let (out, report) = rebind(&write_glb(&json, &bin), &game, &[]).unwrap();
        assert_eq!(report.joints.iter().filter(|(_, s)| s.is_some()).count(), 3);
        assert!(report.extra.is_empty());
        let back = read_glb(&out).unwrap();
        let p = read_floats(&back, 0).unwrap();
        // The head vertex at the game's head: 150 units, 1.5 m.
        assert!((p[7] - 1.5).abs() < 1e-3, "{p:?}");
        let skin = &back.json["skins"][0];
        let names: Vec<&str> = skin["joints"].as_array().unwrap().iter().map(|j| back.json["nodes"][j.as_u64().unwrap() as usize]["name"].as_str().unwrap()).collect();
        assert_eq!(names, vec!["Hip", "Spine1", "Head"]);
    }
}
