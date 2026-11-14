//! The smallest Orrery game: a ball falls and bounces, then the run is rolled
//! back and replayed to the same checksum.
//!
//! `cargo run -p orr_sim --example hello`
use bytemuck::{Pod, Zeroable};
use orr_sim::{fn_systems, fp, ComponentRegistryBuilder, Frame, Game, SimContext, Simulation, System, TickInputs, FP};

const GRAVITY: FP = fp!(0.05);
const RESTITUTION: FP = fp!(0.8);
const TICK_RATE: u32 = 60;
const SEED: u64 = 7;
const SNAPSHOT_TICK: u64 = 60;
const LAST_TICK: u64 = 180;
const PRINT_EVERY: u64 = 30;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Ball {
    height: FP,
    speed: FP,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Bounces {
    count: u32,
}

struct Hello;

impl Game for Hello {
    type Input = u32;
    type Command = ();
    type Event = ();
    type Config = ();

    fn register(builder: &mut ComponentRegistryBuilder) {
        builder.register_component::<Ball>("Ball");
        builder.register_singleton::<Bounces>("Bounces");
    }

    fn setup(frame: &mut Frame, _config: &()) {
        let ball = frame.spawn();
        frame.add(ball, Ball { height: fp!(10), speed: FP::ZERO });
        frame.set_singleton(Bounces { count: 0 });
    }

    fn systems() -> Vec<Box<dyn System<Self>>> {
        fn_systems([fall, bounce])
    }
}

fn fall(ctx: &mut SimContext<Hello>) {
    for (_, (ball,)) in ctx.frame.query::<(&mut Ball,)>() {
        ball.speed -= GRAVITY;
        ball.height += ball.speed;
    }
}

fn bounce(ctx: &mut SimContext<Hello>) {
    let mut bounced = 0;
    for (_, (ball,)) in ctx.frame.query::<(&mut Ball,)>() {
        if ball.height < FP::ZERO {
            ball.height = -ball.height;
            ball.speed = -ball.speed * RESTITUTION;
            bounced += 1;
        }
    }
    ctx.frame.singleton_mut::<Bounces>().count += bounced;
}

fn run_until(sim: &mut Simulation<Hello>, last: u64, mut on_tick: impl FnMut(&Simulation<Hello>)) {
    while sim.tick() < last {
        sim.step(&TickInputs::new(sim.tick() + 1, 1));
        on_tick(sim);
    }
}

fn report(sim: &Simulation<Hello>) {
    if sim.tick() % PRINT_EVERY != 0 {
        return;
    }
    let height = sim.frame().dense::<Ball>().1[0].height;
    let bounces = sim.frame().singleton::<Bounces>().count;
    println!("tick {:3}  height {height}  bounces {bounces}  checksum {:016x}", sim.tick(), sim.checksum());
}

fn main() {
    let mut sim = Simulation::<Hello>::new((), TICK_RATE, SEED);
    run_until(&mut sim, SNAPSHOT_TICK, report);
    let snapshot = sim.frame().clone();
    run_until(&mut sim, LAST_TICK, report);
    let first = sim.checksum();

    sim.restore(&snapshot);
    run_until(&mut sim, LAST_TICK, |_| {});
    assert_eq!(sim.checksum(), first);
    println!("rolled back to tick {SNAPSHOT_TICK} and replayed: same checksum");

    let mut fresh = Simulation::<Hello>::new((), TICK_RATE, SEED);
    run_until(&mut fresh, LAST_TICK, |_| {});
    assert_eq!(fresh.checksum(), first);
    println!("a fresh run with seed {SEED}: same checksum");
}
