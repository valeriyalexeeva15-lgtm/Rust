use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum NodeKind {
    Intersection,
    Entry,
    Exit,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    pub id: String,
    pub x: f64,
    pub y: f64,
    pub kind: NodeKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Segment {
    pub id: String,
    pub from: String,
    pub to: String,
    pub lanes: u32,
    pub length: f64,
    pub speed_limit: f64,
    #[serde(default = "one")]
    pub priority: f64,
    #[serde(default)]
    pub closed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrafficLight {
    pub node_id: String,
    pub phases: Vec<Vec<String>>,
    pub phase_len: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VehicleKind {
    pub name: String,
    pub max_speed: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Spawn {
    pub node_id: String,
    pub rate: f64,
    pub route: Vec<String>,
    #[serde(default)]
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Network {
    pub nodes: Vec<Node>,
    pub segments: Vec<Segment>,
    #[serde(default)]
    pub lights: Vec<TrafficLight>,
    #[serde(default)]
    pub spawns: Vec<Spawn>,
    #[serde(default)]
    pub vehicle_kinds: Vec<VehicleKind>,
}

fn one() -> f64 {
    1.0
}

#[derive(Debug)]
pub enum ModelError {
    Io(String),
    Parse(String),
    Validation(String),
}

impl std::fmt::Display for ModelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ModelError::Io(m) => write!(f, "Ошибка чтения файла: {m}"),
            ModelError::Parse(m) => write!(f, "Ошибка разбора JSON: {m}"),
            ModelError::Validation(m) => write!(f, "Ошибка валидации модели: {m}"),
        }
    }
}

impl Network {
    pub fn load(path: &str) -> Result<Network, ModelError> {
        let text = std::fs::read_to_string(path).map_err(|e| ModelError::Io(e.to_string()))?;
        let net: Network = serde_json::from_str(&text).map_err(|e| ModelError::Parse(e.to_string()))?;
        net.validate()?;
        Ok(net)
    }

    pub fn validate(&self) -> Result<(), ModelError> {
        let err = |m: String| Err(ModelError::Validation(m));

        if self.nodes.is_empty() {
            return err("в сети нет ни одного узла".into());
        }
        if self.segments.is_empty() {
            return err("в сети нет ни одного сегмента".into());
        }

        let mut node_ids: HashMap<&str, &NodeKind> = HashMap::new();
        for n in &self.nodes {
            if node_ids.insert(n.id.as_str(), &n.kind).is_some() {
                return err(format!("повторяющийся id узла: {}", n.id));
            }
        }

        let mut seg_ids: HashMap<&str, &Segment> = HashMap::new();
        for s in &self.segments {
            if seg_ids.insert(s.id.as_str(), s).is_some() {
                return err(format!("повторяющийся id сегмента: {}", s.id));
            }
            if !node_ids.contains_key(s.from.as_str()) {
                return err(format!("сегмент {} ссылается на несуществующий узел {}", s.id, s.from));
            }
            if !node_ids.contains_key(s.to.as_str()) {
                return err(format!("сегмент {} ссылается на несуществующий узел {}", s.id, s.to));
            }
            if s.from == s.to {
                return err(format!("сегмент {} ведёт из узла в самого себя", s.id));
            }
            if s.length <= 0.0 {
                return err(format!("у сегмента {} недопустимая длина {}", s.id, s.length));
            }
            if s.lanes < 1 {
                return err(format!("у сегмента {} должно быть хотя бы 1 полоса", s.id));
            }
            if s.speed_limit < 0.0 {
                return err(format!("у сегмента {} отрицательная скорость", s.id));
            }
            if s.priority <= 0.0 {
                return err(format!("у сегмента {} недопустимый приоритет {}", s.id, s.priority));
            }
        }

        for l in &self.lights {
            if !node_ids.contains_key(l.node_id.as_str()) {
                return err(format!("светофор ссылается на несуществующий узел {}", l.node_id));
            }
            if l.phases.is_empty() {
                return err(format!("у светофора на узле {} нет фаз", l.node_id));
            }
            if l.phase_len <= 0.0 {
                return err(format!("у светофора на узле {} недопустимая длина фазы", l.node_id));
            }
            for phase in &l.phases {
                for sid in phase {
                    match seg_ids.get(sid.as_str()) {
                        None => return err(format!("светофор {} ссылается на несуществующий сегмент {}", l.node_id, sid)),
                        Some(seg) if seg.to != l.node_id => {
                            return err(format!("светофор {} управляет сегментом {}, который в него не входит", l.node_id, sid));
                        }
                        _ => {}
                    }
                }
            }
        }

        for sp in &self.spawns {
            match node_ids.get(sp.node_id.as_str()) {
                None => return err(format!("источник потока ссылается на несуществующий узел {}", sp.node_id)),
                Some(kind) if **kind != NodeKind::Entry => {
                    return err(format!("источник потока должен стартовать с узла-въезда, а {} им не является", sp.node_id));
                }
                _ => {}
            }
            if sp.rate < 0.0 {
                return err(format!("отрицательная интенсивность у источника на узле {}", sp.node_id));
            }
            if sp.route.is_empty() {
                return err(format!("пустой маршрут у источника на узле {}", sp.node_id));
            }
            let mut prev_to: Option<&str> = None;
            for sid in &sp.route {
                let seg = match seg_ids.get(sid.as_str()) {
                    Some(s) => *s,
                    None => return err(format!("маршрут с узла {} ссылается на несуществующий сегмент {}", sp.node_id, sid)),
                };
                if let Some(prev) = prev_to {
                    if seg.from != prev {
                        return err(format!("маршрут с узла {} разорван: сегмент {} не продолжает предыдущий", sp.node_id, sid));
                    }
                }
                prev_to = Some(&seg.to);
            }
            let first = &sp.route[0];
            if let Some(seg) = seg_ids.get(first.as_str()) {
                if seg.from != sp.node_id {
                    return err(format!("первый сегмент маршрута {} не выходит из узла въезда {}", first, sp.node_id));
                }
            }
        }

        Ok(())
    }
}
