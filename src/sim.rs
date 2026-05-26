use crate::model::{Network, NodeKind};
use std::collections::HashMap;

const DT: f64 = 1.0;
const MIN_GAP: f64 = 8.0;

#[derive(Clone, Copy, PartialEq)]
pub enum Load {
    Free,
    Loaded,
    Jam,
}

struct Vehicle {
    route: Vec<usize>,
    ptr: usize,
    pos: f64,
    max_speed: f64,
    t_spawn: f64,
}

struct LightRt {
    node: String,
    phases: Vec<Vec<usize>>,
    phase_len: f64,
    cur: usize,
    timer: f64,
}

pub struct Report {
    pub scenario: String,
    pub completed: u64,
    pub avg_travel: f64,
    pub avg_speed: f64,
    pub top_segments: Vec<(String, f64)>,
    pub jam_count: u64,
    pub jam_duration: u64,
    pub sim_time: f64,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Scenario {
    Basic,
    Rush,
    Closed,
    Lights,
}

impl Scenario {
    pub fn name(&self) -> &'static str {
        match self {
            Scenario::Basic => "Базовый",
            Scenario::Rush => "Час пик",
            Scenario::Closed => "Перекрытие",
            Scenario::Lights => "Светофоры",
        }
    }
}

pub struct Sim {
    pub net: Network,
    pub scenario: Scenario,
    pub time: f64,
    pub steps: u64,

    seg_index: HashMap<String, usize>,
    vehicles: Vec<Vehicle>,
    lights: Vec<LightRt>,
    spawn_acc: Vec<f64>,

    seg_load: Vec<f64>,
    seg_accum: Vec<f64>,
    seg_jam: Vec<bool>,

    completed: u64,
    sum_travel: f64,
    sum_speed: f64,
    jam_count: u64,
    jam_duration: u64,
}

impl Sim {
    pub fn new(mut net: Network, scenario: Scenario) -> Sim {
        apply_scenario(&mut net, scenario);

        let seg_index: HashMap<String, usize> = net
            .segments
            .iter()
            .enumerate()
            .map(|(i, s)| (s.id.clone(), i))
            .collect();

        let lights = net
            .lights
            .iter()
            .map(|l| LightRt {
                node: l.node_id.clone(),
                phases: l
                    .phases
                    .iter()
                    .map(|ph| ph.iter().filter_map(|s| seg_index.get(s).copied()).collect())
                    .collect(),
                phase_len: l.phase_len,
                cur: 0,
                timer: 0.0,
            })
            .collect();

        let n = net.segments.len();
        let spawn_acc = vec![0.0; net.spawns.len()];

        Sim {
            net,
            scenario,
            time: 0.0,
            steps: 0,
            seg_index,
            vehicles: Vec::new(),
            lights,
            spawn_acc,
            seg_load: vec![0.0; n],
            seg_accum: vec![0.0; n],
            seg_jam: vec![false; n],
            completed: 0,
            sum_travel: 0.0,
            sum_speed: 0.0,
            jam_count: 0,
            jam_duration: 0,
        }
    }

    pub fn step(&mut self) {
        self.spawn();
        self.recompute_load();
        self.move_vehicles();
        self.advance_lights();
        self.collect_stats();
        self.time += DT;
        self.steps += 1;
    }

    fn spawn(&mut self) {
        for (i, sp) in self.net.spawns.iter().enumerate() {
            self.spawn_acc[i] = (self.spawn_acc[i] + sp.rate).min(5.0);
            while self.spawn_acc[i] >= 1.0 {
                let route: Vec<usize> = sp.route.iter().filter_map(|s| self.seg_index.get(s).copied()).collect();
                if route.is_empty() || self.net.segments[route[0]].closed {
                    break;
                }
                let max_speed = self
                    .net
                    .vehicle_kinds
                    .iter()
                    .find(|k| k.name == sp.kind)
                    .or_else(|| self.net.vehicle_kinds.first())
                    .map(|k| k.max_speed)
                    .unwrap_or(f64::INFINITY);
                self.vehicles.push(Vehicle { route, ptr: 0, pos: 0.0, max_speed, t_spawn: self.time });
                self.spawn_acc[i] -= 1.0;
            }
        }
    }

    fn recompute_load(&mut self) {
        for v in self.seg_load.iter_mut() {
            *v = 0.0;
        }
        for veh in &self.vehicles {
            let seg = veh.route[veh.ptr];
            self.seg_load[seg] += 1.0;
        }
    }

    fn capacity(&self, seg: usize) -> f64 {
        let s = &self.net.segments[seg];
        (s.lanes as f64 * s.length / MIN_GAP).max(1.0)
    }

    fn move_vehicles(&mut self) {
        let mut survivors: Vec<Vehicle> = Vec::with_capacity(self.vehicles.len());
        let vehicles = std::mem::take(&mut self.vehicles);

        for mut veh in vehicles {
            let seg = veh.route[veh.ptr];
            let (length, speed_limit, closed, to) = {
                let s = &self.net.segments[seg];
                (s.length, s.speed_limit, s.closed, s.to.clone())
            };

            let density = (self.seg_load[seg] / self.capacity(seg)).min(1.0);
            let factor = if closed { 0.0 } else { (1.0 - density).max(0.0) };
            let speed = (speed_limit * factor).min(veh.max_speed);
            veh.pos += speed * DT;

            if veh.pos >= length {
                if veh.ptr + 1 >= veh.route.len() {
                    let dist: f64 = veh.route.iter().map(|&i| self.net.segments[i].length).sum();
                    let travel = (self.time - veh.t_spawn).max(DT);
                    self.completed += 1;
                    self.sum_travel += travel;
                    self.sum_speed += dist / travel;
                    continue;
                }
                if self.is_green(&to, seg) {
                    veh.pos = 0.0;
                    veh.ptr += 1;
                } else {
                    veh.pos = length;
                }
            }
            survivors.push(veh);
        }
        self.vehicles = survivors;
    }

    fn is_green(&self, node: &str, seg: usize) -> bool {
        match self.lights.iter().find(|l| l.node == node) {
            None => true,
            Some(l) => l.phases.get(l.cur).map_or(true, |ph| ph.contains(&seg)),
        }
    }

    fn advance_lights(&mut self) {
        for l in &mut self.lights {
            l.timer += DT;
            if l.timer >= l.phase_len {
                l.timer = 0.0;
                l.cur = (l.cur + 1) % l.phases.len();
            }
        }
    }

    fn collect_stats(&mut self) {
        for i in 0..self.net.segments.len() {
            self.seg_accum[i] += self.seg_load[i];
            let jam = self.load_state(i) == Load::Jam;
            if jam {
                self.jam_duration += 1;
                if !self.seg_jam[i] {
                    self.jam_count += 1;
                }
            }
            self.seg_jam[i] = jam;
        }
    }

    pub fn load_state(&self, seg: usize) -> Load {
        if self.net.segments[seg].closed {
            return Load::Jam;
        }
        let ratio = self.seg_load[seg] / self.capacity(seg);
        if ratio < 0.5 {
            Load::Free
        } else if ratio < 0.85 {
            Load::Loaded
        } else {
            Load::Jam
        }
    }

    pub fn active(&self) -> usize {
        self.vehicles.len()
    }

    pub fn vehicle_points(&self) -> Vec<(f64, f64)> {
        let mut pts = Vec::with_capacity(self.vehicles.len());
        for veh in &self.vehicles {
            let s = &self.net.segments[veh.route[veh.ptr]];
            let (a, b) = (self.node_xy(&s.from), self.node_xy(&s.to));
            let t = (veh.pos / s.length).clamp(0.0, 1.0);
            pts.push((a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t));
        }
        pts
    }

    fn node_xy(&self, id: &str) -> (f64, f64) {
        self.net
            .nodes
            .iter()
            .find(|n| n.id == id)
            .map(|n| (n.x, n.y))
            .unwrap_or((0.0, 0.0))
    }

    pub fn scale_rates(&mut self, factor: f64) {
        for sp in &mut self.net.spawns {
            sp.rate = (sp.rate * factor).clamp(0.0, 5.0);
        }
    }

    pub fn toggle_closed(&mut self, seg: usize) {
        if let Some(s) = self.net.segments.get_mut(seg) {
            s.closed = !s.closed;
        }
    }

    pub fn cycle_light_mode(&mut self) {
        for l in &mut self.lights {
            l.phase_len = match l.phase_len as i64 {
                0..=4 => 8.0,
                5..=9 => 14.0,
                _ => 3.0,
            };
        }
    }

    pub fn report(&self) -> Report {
        let avg_travel = if self.completed > 0 { self.sum_travel / self.completed as f64 } else { 0.0 };
        let avg_speed = if self.completed > 0 { self.sum_speed / self.completed as f64 } else { 0.0 };

        let mut idx: Vec<usize> = (0..self.net.segments.len()).collect();
        idx.sort_by(|&a, &b| self.seg_accum[b].partial_cmp(&self.seg_accum[a]).unwrap());
        let top_segments = idx
            .iter()
            .take(3)
            .map(|&i| {
                let avg = if self.steps > 0 { self.seg_accum[i] / self.steps as f64 } else { 0.0 };
                (self.net.segments[i].id.clone(), avg)
            })
            .collect();

        Report {
            scenario: self.scenario.name().to_string(),
            completed: self.completed,
            avg_travel,
            avg_speed,
            top_segments,
            jam_count: self.jam_count,
            jam_duration: self.jam_duration,
            sim_time: self.time,
        }
    }
}

fn apply_scenario(net: &mut Network, scenario: Scenario) {
    match scenario {
        Scenario::Basic => {}
        Scenario::Rush => {
            for sp in &mut net.spawns {
                sp.rate = (sp.rate * 2.5).min(5.0);
            }
        }
        Scenario::Closed => {
            let entries: Vec<String> = net
                .nodes
                .iter()
                .filter(|n| n.kind == NodeKind::Entry)
                .map(|n| n.id.clone())
                .collect();
            if let Some(s) = net.segments.iter_mut().find(|s| entries.contains(&s.from)) {
                s.closed = true;
            }
        }
        Scenario::Lights => {
            for l in &mut net.lights {
                l.phase_len *= 2.0;
            }
        }
    }
}
