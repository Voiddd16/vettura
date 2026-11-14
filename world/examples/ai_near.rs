//! 找 AI 线上离给定坐标最近的点：`ai_near <fast_lane.ai> <x> <z> [半径]`
use vettura_world::AiSpline;

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(path) = args.next() else {
        eprintln!("用法: ai_near <fast_lane.ai> <x> <z> [半径_m]");
        std::process::exit(2);
    };
    let Some(x) = args.next().and_then(|v| v.parse::<f32>().ok()) else {
        eprintln!("x 无效");
        std::process::exit(2);
    };
    let Some(z) = args.next().and_then(|v| v.parse::<f32>().ok()) else {
        eprintln!("z 无效");
        std::process::exit(2);
    };
    let radius = args
        .next()
        .and_then(|v| v.parse::<f32>().ok())
        .unwrap_or(30.0);

    let line = AiSpline::from_path(&path).expect("解析失败");
    let target = glam::Vec3::new(x, 0.0, z);
    let length = line.lap_length_m();
    println!("lap_length={length:.1} target=({x}, {z}) radius={radius}");

    let mut distance = 0.0f32;
    while distance < length {
        if let Some(sample) = line.sample_at_distance(distance) {
            let d = sample.position.distance(target);
            if d < radius {
                println!(
                    "d={:>7.1} pos=({:>8.1},{:>8.1}) dist_to_target={:>6.1} v={:>6.2} m/s ({:>6.1} km/h)",
                    distance,
                    sample.position.x,
                    sample.position.z,
                    d,
                    sample.speed_mps.unwrap_or(f32::NAN),
                    sample.speed_mps.unwrap_or(0.0) * 3.6,
                );
            }
        }
        distance += 2.0;
    }
}
