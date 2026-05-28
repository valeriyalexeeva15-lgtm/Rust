mod model;
mod sim;

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    symbols::Marker,
    text::{Line, Span},
    widgets::{
        canvas::{Canvas, Line as CanvasLine, Points},
        Block, Borders, Paragraph, Wrap,
    },
    Frame,
};
use sim::{Load, Report, Scenario, Sim};
use std::time::{Duration, Instant};

const DEFAULT_FILE: &str = "scenarios/city.json";

#[derive(PartialEq)]
enum RunState {
    Running,
    Paused,
    Stopped,
}

struct App {
    file: String,
    base: model::Network,
    sim: Sim,
    state: RunState,
    speed: u32,
    cam_x: f64,
    cam_y: f64,
    zoom: f64,
    status: String,
    history: Vec<Report>,
    cursor: usize,
    show_report: bool,
}

impl App {
    fn new(file: String) -> Result<App, String> {
        let base = model::Network::load(&file).map_err(|e| e.to_string())?;
        let sim = Sim::new(base.clone(), Scenario::Basic);
        let (cam_x, cam_y, zoom) = camera_for(&base);
        Ok(App {
            file,
            base,
            sim,
            state: RunState::Paused,
            speed: 4,
            cam_x,
            cam_y,
            zoom,
            status: "Готово. Space — старт.".into(),
            history: Vec::new(),
            cursor: 0,
            show_report: false,
        })
    }

    fn set_scenario(&mut self, scen: Scenario) {
        self.sim = Sim::new(self.base.clone(), scen);
        self.state = RunState::Paused;
        self.show_report = false;
        self.status = format!("Сценарий: {}", scen.name());
    }

    fn reload_file(&mut self) {
        match model::Network::load(&self.file) {
            Ok(net) => {
                self.base = net;
                let scen = self.sim.scenario;
                self.sim = Sim::new(self.base.clone(), scen);
                self.state = RunState::Paused;
                self.show_report = false;
                self.status = format!("Файл перезагружен: {}", self.file);
            }
            Err(e) => {
                self.status = format!("ОШИБКА: {e}");
            }
        }
    }

    fn stop(&mut self) {
        if self.state != RunState::Stopped {
            self.history.push(self.sim.report());
        }
        self.state = RunState::Stopped;
        self.show_report = true;
        self.status = "Симуляция остановлена. Отчёт сформирован.".into();
    }
}

fn camera_for(net: &model::Network) -> (f64, f64, f64) {
    let mut minx = f64::INFINITY;
    let mut maxx = f64::NEG_INFINITY;
    let mut miny = f64::INFINITY;
    let mut maxy = f64::NEG_INFINITY;
    for n in &net.nodes {
        minx = minx.min(n.x);
        maxx = maxx.max(n.x);
        miny = miny.min(n.y);
        maxy = maxy.max(n.y);
    }
    let cx = (minx + maxx) / 2.0;
    let cy = (miny + maxy) / 2.0;
    let span = (maxx - minx).max(maxy - miny).max(1.0);
    (cx, cy, span * 0.6)
}

fn node_xy(net: &model::Network, id: &str) -> (f64, f64) {
    net.nodes
        .iter()
        .find(|n| n.id == id)
        .map(|n| (n.x, n.y))
        .unwrap_or((0.0, 0.0))
}

fn main() {
    let file = std::env::args().nth(1).unwrap_or_else(|| DEFAULT_FILE.to_string());
    let mut app = match App::new(file) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("Не удалось загрузить модель:\n{e}");
            std::process::exit(1);
        }
    };
    let mut terminal = ratatui::init();
    let res = run(&mut terminal, &mut app);
    ratatui::restore();
    if let Err(e) = res {
        eprintln!("Ошибка выполнения: {e}");
    }
}

fn run(terminal: &mut ratatui::DefaultTerminal, app: &mut App) -> std::io::Result<()> {
    let tick = Duration::from_millis(60);
    let mut last = Instant::now();
    loop {
        terminal.draw(|f| ui(f, app))?;
        let timeout = tick.saturating_sub(last.elapsed());
        if event::poll(timeout)? {
            if let Event::Key(k) = event::read()? {
                if k.kind == KeyEventKind::Press && handle_key(app, k.code) {
                    break;
                }
            }
        }
        if last.elapsed() >= tick {
            if app.state == RunState::Running {
                for _ in 0..app.speed {
                    app.sim.step();
                }
            }
            last = Instant::now();
        }
    }
    Ok(())
}

fn handle_key(app: &mut App, code: KeyCode) -> bool {
    let pan = app.zoom * 0.1;
    match code {
        KeyCode::Char('q') | KeyCode::Char('Q') => return true,
        KeyCode::Char(' ') => {
            app.state = match app.state {
                RunState::Running => RunState::Paused,
                _ => {
                    app.show_report = false;
                    RunState::Running
                }
            };
        }
        KeyCode::Char('s') | KeyCode::Char('S') => app.stop(),
        KeyCode::Char('r') | KeyCode::Char('R') => {
            let s = app.sim.scenario;
            app.set_scenario(s);
        }
        KeyCode::Char('1') => app.set_scenario(Scenario::Basic),
        KeyCode::Char('2') => app.set_scenario(Scenario::Rush),
        KeyCode::Char('3') => app.set_scenario(Scenario::Closed),
        KeyCode::Char('4') => app.set_scenario(Scenario::Lights),
        KeyCode::Char('+') | KeyCode::Char('=') => app.speed = (app.speed + 1).min(20),
        KeyCode::Char('-') | KeyCode::Char('_') => app.speed = app.speed.saturating_sub(1).max(1),
        KeyCode::Char('.') => app.sim.scale_rates(1.3),
        KeyCode::Char(',') => app.sim.scale_rates(0.77),
        KeyCode::Left => app.cam_x -= pan,
        KeyCode::Right => app.cam_x += pan,
        KeyCode::Up => app.cam_y += pan,
        KeyCode::Down => app.cam_y -= pan,
        KeyCode::Char('[') => app.zoom *= 1.2,
        KeyCode::Char(']') => app.zoom /= 1.2,
        KeyCode::Tab => {
            if !app.sim.net.segments.is_empty() {
                app.cursor = (app.cursor + 1) % app.sim.net.segments.len();
            }
        }
        KeyCode::Char('c') | KeyCode::Char('C') => {
            app.sim.toggle_closed(app.cursor);
            app.status = format!("Переключён сегмент {}", app.sim.net.segments[app.cursor].id);
        }
        KeyCode::Char('l') | KeyCode::Char('L') => {
            app.sim.cycle_light_mode();
            app.status = "Режим светофоров изменён".into();
        }
        KeyCode::Char('o') | KeyCode::Char('O') => app.reload_file(),
        _ => {}
    }
    false
}

fn ui(f: &mut Frame, app: &App) {
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(5), Constraint::Length(4)])
        .split(f.area());
    let top = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(65), Constraint::Percentage(35)])
        .split(root[0]);
    draw_map(f, app, top[0]);
    draw_stats(f, app, top[1]);
    draw_hotkeys(f, app, root[1]);
}

fn draw_map(f: &mut Frame, app: &App, area: Rect) {
    let sim = &app.sim;
    let cursor = app.cursor;
    let bounds_x = [app.cam_x - app.zoom, app.cam_x + app.zoom];
    let bounds_y = [app.cam_y - app.zoom, app.cam_y + app.zoom];

    let canvas = Canvas::default()
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Сеть  [зелёный свободно | жёлтый загружено | красный затор]"),
        )
        .marker(Marker::Braille)
        .x_bounds(bounds_x)
        .y_bounds(bounds_y)
        .paint(move |ctx| {
            let net = &sim.net;
            for (i, s) in net.segments.iter().enumerate() {
                let a = node_xy(net, &s.from);
                let b = node_xy(net, &s.to);
                let color = if i == cursor {
                    Color::Magenta
                } else {
                    match sim.load_state(i) {
                        Load::Free => Color::Green,
                        Load::Loaded => Color::Yellow,
                        Load::Jam => Color::Red,
                    }
                };
                ctx.draw(&CanvasLine {
                    x1: a.0,
                    y1: a.1,
                    x2: b.0,
                    y2: b.1,
                    color,
                });
            }
            ctx.layer();
            let pts = sim.vehicle_points();
            ctx.draw(&Points {
                coords: &pts,
                color: Color::White,
            });
            ctx.layer();
            for n in &net.nodes {
                let (mark, col) = match n.kind {
                    model::NodeKind::Entry => ("E", Color::Cyan),
                    model::NodeKind::Exit => ("X", Color::Cyan),
                    model::NodeKind::Intersection => ("+", Color::Gray),
                };
                ctx.print(n.x, n.y, Span::styled(mark, Style::default().fg(col)));
            }
        });
    f.render_widget(canvas, area);
}

fn draw_stats(f: &mut Frame, app: &App, area: Rect) {
    let sim = &app.sim;
    let r = sim.report();
    let state = match app.state {
        RunState::Running => "идёт",
        RunState::Paused => "пауза",
        RunState::Stopped => "стоп",
    };
    let mut lines = vec![
        Line::from(format!("Сценарий  : {}", sim.scenario.name())),
        Line::from(format!("Состояние : {}", state)),
        Line::from(format!("Время     : {:.0} c", sim.time)),
        Line::from(format!("Шаг       : {}", sim.steps)),
        Line::from(format!("Скорость  : x{}", app.speed)),
        Line::from(format!("В сети    : {} ТС", sim.active())),
        Line::from(format!("Обработано: {} ТС", r.completed)),
        Line::from(""),
        Line::from(format!("Ср. скорость: {:.1} м/с", r.avg_speed)),
        Line::from(format!("Ср. время   : {:.1} c", r.avg_travel)),
        Line::from(format!("Заторы      : {} ({} шагов)", r.jam_count, r.jam_duration)),
        Line::from(""),
        Line::from("Загруженные участки:"),
    ];
    for (id, v) in &r.top_segments {
        lines.push(Line::from(format!("  {} : {:.1} ТС", id, v)));
    }
    if app.show_report && !app.history.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "=== Сравнение сценариев ===",
            Style::default().fg(Color::Cyan),
        )));
        for h in &app.history {
            lines.push(Line::from(format!(
                "{} (за {:.0}c): ТС {}, v {:.1} м/с, заторы {}",
                h.scenario, h.sim_time, h.completed, h.avg_speed, h.jam_count
            )));
        }
    }
    let p = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title("Статистика"))
        .wrap(Wrap { trim: true });
    f.render_widget(p, area);
}

fn draw_hotkeys(f: &mut Frame, app: &App, area: Rect) {
    let keys = "Space старт/пауза  S стоп+отчёт  R сброс  1-4 сценарий  +/- скорость  ,/. интенсивность  Tab выбор сегмента  C перекрыть  L светофор  стрелки сдвиг  [ ] зум  O файл  Q выход";
    let lines = vec![
        Line::from(Span::styled(keys, Style::default().fg(Color::Gray))),
        Line::from(Span::styled(
            format!("> {}", app.status),
            Style::default().fg(Color::Yellow),
        )),
    ];
    let p = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title("Управление"))
        .wrap(Wrap { trim: true });
    f.render_widget(p, area);
}
