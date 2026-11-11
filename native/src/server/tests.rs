//! Pruebas del relé (lo que no necesita red de verdad; lo demás está en tests/).

use super::groups::{GROUPS_MAX, GROUPS_PER_OWNER};
use super::*;

fn server() -> VoiceServer {
    VoiceServer::start("127.0.0.1:0".parse().unwrap(), 48.0).unwrap()
}

fn kinds(s: &VoiceServer) -> Vec<(EventKind, String)> {
    s.poll_events().into_iter().map(|e| (e.kind, e.detail)).collect()
}

#[test]
fn groups_create_join_leave_and_auto_cleanup() {
    let s = server();
    let (a, b) = ([1; 16], [2; 16]);
    s.add_player(a);
    s.add_player(b);
    assert_eq!(s.create_group(&a, "Equipo Rojo", Some("1234")), Ok(()));
    assert_eq!(s.create_group(&b, "equipo rojo", None), Err(GroupError::AlreadyExists));
    assert_eq!(s.join_group(&b, "EQUIPO ROJO", None), Err(GroupError::WrongPassword));
    assert_eq!(s.join_group(&b, "equipo rojo", Some("1234")), Ok(()));
    assert_eq!(s.group_of(&b).as_deref(), Some("Equipo Rojo"));
    assert_eq!(s.group_owner("equipo rojo"), Some(a));
    assert_eq!(s.list_groups(), vec![("Equipo Rojo".to_string(), 2, true, false)]);
    assert!(s.leave_group(&a));
    assert_eq!(s.group_owner("equipo rojo"), Some(b), "el dueño se va: la corona pasa a otro miembro");
    assert!(s.leave_group(&b));
    assert!(s.list_groups().is_empty(), "un grupo temporal vacío desaparece");
    assert_eq!(
        kinds(&s),
        vec![
            (EventKind::Joined, "Equipo Rojo".into()),
            (EventKind::Left, "Equipo Rojo".into()),
            (EventKind::NewOwner, "Equipo Rojo".into()),
            (EventKind::Left, "Equipo Rojo".into()),
        ]
    );
    assert_eq!(s.create_group(&a, "   ", None), Err(GroupError::InvalidName));
    assert_eq!(s.create_group(&a, "con\ttab", None), Err(GroupError::InvalidName));
    assert_eq!(s.create_group(&a, "a|b", None), Err(GroupError::InvalidName));
    assert_eq!(s.create_group(&a, "ok", Some("pa|ss")), Ok(()), "la contraseña sí puede llevar '|'");
}

#[test]
fn kick_requires_owner_or_admin() {
    let s = server();
    let (owner, member, other) = ([1; 16], [2; 16], [3; 16]);
    for p in [owner, member, other] {
        s.add_player(p);
    }
    s.create_group(&owner, "club", None).unwrap();
    s.join_group(&member, "club", None).unwrap();
    s.join_group(&other, "club", None).unwrap();
    assert_eq!(s.kick_from_group(&other, &member, false), Err(GroupError::NotOwner));
    assert_eq!(s.kick_from_group(&owner, &member, false), Ok("club".into()));
    assert_eq!(s.group_of(&member), None);
    assert_eq!(s.kick_from_group(&member, &other, true), Ok("club".into()), "un admin puede");
    assert_eq!(s.kick_from_group(&owner, &other, false), Err(GroupError::NotInGroup));
}

#[test]
fn persistent_groups_and_mutes_survive_a_restart() {
    let dir = std::env::temp_dir().join(format!("ferrovoz-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("data.txt");
    let _ = std::fs::remove_file(&file);
    let (a, b, troll) = ([1; 16], [2; 16], [9; 16]);
    {
        let s = server();
        s.set_data_file(&file).unwrap();
        s.add_player(a);
        s.add_player(b);
        s.create_group(&a, "Piratas", Some("loro")).unwrap();
        s.join_group(&b, "piratas", Some("loro")).unwrap();
        assert_eq!(s.set_group_persistent(&b, true, false), Err(GroupError::NotOwner));
        assert_eq!(s.set_group_persistent(&a, true, false), Ok("Piratas".into()));
        s.set_muted(&troll, true);
        s.remove_player(&a);
        s.remove_player(&b);
        assert_eq!(s.list_groups(), vec![("Piratas".to_string(), 0, true, true)], "permanente aunque se vacíe");
    }
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.contains("group\tPiratas\t01010101-0101-0101-0101-010101010101\tpbkdf2-sha256$"), "{text}");
    assert!(!text.contains("loro"), "la contraseña no se guarda tal cual: {text}");
    // Reinicio: servidor nuevo con el mismo fichero.
    let s = server();
    s.set_data_file(&file).unwrap();
    assert!(s.is_muted(&troll), "el silencio sobrevive al reinicio");
    assert_eq!(s.list_groups().len(), 1);
    s.add_player(b);
    assert_eq!(s.group_of(&b).as_deref(), Some("Piratas"), "al volver entra solo en su grupo permanente");
    assert_eq!(kinds(&s), vec![(EventKind::Rejoined, "Piratas".into())]);
    assert_eq!(s.group_owner("piratas"), Some(a), "el dueño se mantiene aunque no esté conectado");
    s.add_player(troll);
    assert_eq!(s.join_group(&troll, "piratas", Some("Loro")), Err(GroupError::WrongPassword));
    assert_eq!(s.join_group(&troll, "piratas", Some("loro")), Ok(()), "la contraseña sigue valiendo tras reiniciar");
    assert_eq!(s.delete_group(&b, "piratas", false), Err(GroupError::NotOwner));
    assert_eq!(s.delete_group(&b, "piratas", true), Ok("Piratas".into()));
    // Se escribe en segundo plano: llega al disco enseguida.
    let start = Instant::now();
    while std::fs::read_to_string(&file).unwrap().contains("Piratas") {
        assert!(start.elapsed() < Duration::from_secs(5), "el borrado no llega al archivo");
        std::thread::sleep(Duration::from_millis(10));
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn plain_text_passwords_from_older_versions_are_protected_on_load() {
    let dir = std::env::temp_dir().join(format!("ferrovoz-test-legacy-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("data.txt");
    std::fs::write(&file, "group\tClub\t-\tvieja1234\ngroup\tAbierto\t-\t\n").unwrap();
    {
        let s = server();
        s.set_data_file(&file).unwrap();
        s.add_player([5; 16]);
        assert_eq!(s.join_group(&[5; 16], "club", Some("mal")), Err(GroupError::WrongPassword));
        assert_eq!(s.join_group(&[5; 16], "club", Some("vieja1234")), Ok(()));
        assert_eq!(s.group_info("abierto").map(|i| i.1), Some(false), "sin contraseña sigue abierto");
    }
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(!text.contains("vieja1234") && text.contains("pbkdf2-sha256$"), "{text}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn group_names_cannot_carry_chat_formatting() {
    let s = server();
    s.add_player([1; 16]);
    assert_eq!(s.create_group(&[1; 16], "§cRojo", None), Err(GroupError::InvalidName));
    assert_eq!(s.create_group(&[1; 16], "Rojo", Some("con§color")), Ok(()), "en la contraseña da igual: no se muestra");
}

#[test]
fn members_are_told_when_their_group_is_deleted() {
    let s = server();
    let (owner, a, b) = ([1; 16], [2; 16], [3; 16]);
    for p in [owner, a, b] {
        s.add_player(p);
    }
    s.create_group(&owner, "Club", None).unwrap();
    s.join_group(&a, "club", None).unwrap();
    s.join_group(&b, "club", None).unwrap();
    s.poll_events();
    s.delete_group(&owner, "club", false).unwrap();
    let events: Vec<(EventKind, PlayerId)> = s.poll_events().into_iter().map(|e| (e.kind, e.player)).collect();
    assert!(events.contains(&(EventKind::Dissolved, a)) && events.contains(&(EventKind::Dissolved, b)), "{events:?}");
    assert!(!events.contains(&(EventKind::Dissolved, owner)), "quien lo borra ya lo sabe");
    assert!(events.contains(&(EventKind::Deleted, owner)));
}

#[test]
fn a_thread_failing_with_the_lock_held_does_not_take_the_relay_down() {
    let s = server();
    let shared = s.shared.clone();
    let _ = std::thread::spawn(move || {
        let _guard = shared.state.lock().unwrap();
        panic!("fallo a propósito con el lock cogido");
    })
    .join();
    assert!(s.shared.state.is_poisoned());
    // Todo sigue funcionando.
    s.add_player([1; 16]);
    s.create_group(&[1; 16], "Sigue", None).unwrap();
    assert_eq!(s.group_of(&[1; 16]).as_deref(), Some("Sigue"));
}

#[test]
fn loss_meter_measures_gaps() {
    let mut m = LossMeter::default();
    for seq in 0..400u32 {
        if seq % 4 != 3 {
            m.record(seq);
        }
    }
    assert!((20..=30).contains(&m.percent), "≈25 %: {}", m.percent);
    let mut ok = LossMeter::default();
    for seq in 0..400u32 {
        ok.record(seq);
    }
    assert_eq!(ok.percent, 0);
}

#[test]
fn token_bucket_limits_bursts() {
    let mut b = TokenBucket::new(10.0, 5.0);
    let allowed = (0..50).filter(|_| b.take()).count();
    assert_eq!(allowed, 5, "solo la ráfaga inicial");
    std::thread::sleep(Duration::from_millis(220));
    assert!(b.take() && b.take(), "se recarga con el tiempo");
}

#[test]
fn tunnel_rejects_packets_from_another_player() {
    let s = server();
    let (a, b) = ([1; 16], [2; 16]);
    let key_b = s.add_player(b);
    s.add_player(a);
    let hello_b = protocol::seal(&key_b, PacketType::Hello, Some(&b), &[], 1);
    s.handle_tunnel(&a, &hello_b);
    assert_eq!(s.connected_count(), 0);
    s.handle_tunnel(&b, &hello_b);
    assert_eq!(s.tunnel_count(), 1);
    let pending = s.poll_tunnel(&b);
    assert_eq!(pending.len(), 2, "recibe su Pong y su estado de grupo por el túnel");
    // El mismo Hello otra vez: lo rechaza el anti-replay.
    s.handle_tunnel(&b, &hello_b);
    assert_eq!(s.stats().replayed.load(Ordering::Relaxed), 1);
}

/// Lo que cuesta repartir una trama con 500 juntos y 300 hablando (sin red: solo el
/// trabajo del hilo que recibe, con el lock).
#[test]
#[ignore = "medida: cargo test --release --lib route_cost -- --ignored --nocapture"]
fn route_cost_with_500_together() {
    let s = VoiceServer::start_with_receivers("127.0.0.1:0".parse().unwrap(), 48.0, 1).unwrap();
    s.set_max_voices(24);
    let mut keys = Vec::new();
    for i in 0..500u32 {
        let mut id = [0u8; 16];
        id[..4].copy_from_slice(&i.to_le_bytes());
        let key = s.add_player(id);
        s.update_player(&id, [(i % 10) as f64 * 2.0, 64.0, (i / 10) as f64 * 2.0], 1);
        s.state().players.get_mut(&id).unwrap().transport = Transport::Udp("127.0.0.1:9".parse().unwrap());
        if i % 5 < 3 {
            keys.push((id, key));
        }
    }
    let addr: SocketAddr = "127.0.0.1:9".parse().unwrap();
    let mut counter = 10u64;
    let mut send_round = |s: &VoiceServer, seq: u32| -> (Duration, usize) {
        let mut total = Duration::ZERO;
        let mut sent = 0;
        for (id, key) in &keys {
            counter += 1;
            let body = AudioUp { seq, talk: TalkMode::Normal, env: Environment::default(), opus: vec![0xF8; 60] }.encode();
            let pkt = protocol::seal(key, PacketType::AudioUp, Some(id), &body, counter);
            let (mut out, mut jobs) = (Vec::new(), Vec::new());
            let t = Instant::now();
            let _ = s.shared.handle(&pkt, Source::Udp(addr), &mut out, &mut jobs);
            total += t.elapsed();
            sent += jobs.len();
        }
        (total, sent)
    };
    // Que todos hablen y haya plan de multitudes.
    send_round(&s, 1);
    s.shared.plan_crowds();
    let mut best = f64::MAX;
    for seq in 2..8 {
        std::thread::sleep(Duration::from_millis(25)); // el límite por jugador se recarga
        let (t, jobs) = send_round(&s, seq);
        let per_frame = t.as_secs_f64() * 1e6 / keys.len() as f64;
        best = best.min(per_frame);
        println!("ronda {seq}: {per_frame:.1} µs por trama, {:.1} oyentes por trama", jobs as f64 / keys.len() as f64);
    }
    println!("mejor: {best:.1} µs por trama ({:.0} tramas/s en un hilo)", 1e6 / best);
}

/// Sin tope, un jugador podía crear y fijar grupos en bucle: cada uno se queda en el
/// archivo de datos para siempre y encarece cada cambio de grupo posterior.
#[test]
fn nobody_can_leave_the_server_full_of_pinned_groups() {
    let s = server();
    let a = [1; 16];
    s.add_player(a);
    // Crear, fijar y salir: el grupo se queda, con él como dueño.
    for n in 0..GROUPS_PER_OWNER {
        assert_eq!(s.create_group(&a, &format!("suyo {n}"), None), Ok(()), "el número {n}");
        s.set_group_persistent_by_name(&format!("suyo {n}"), true).unwrap();
        assert!(s.leave_group(&a));
    }
    assert_eq!(s.create_group(&a, "uno más", None), Err(GroupError::TooMany), "ya tiene {GROUPS_PER_OWNER}");
    assert_eq!(s.list_groups().len(), GROUPS_PER_OWNER);

    // Y con el servidor lleno tampoco puede otro, aunque no tenga ninguno.
    let b = [2; 16];
    s.add_player(b);
    assert_eq!(s.create_group(&b, "de otro", None), Ok(()), "a este le queda sitio");
    assert!(s.leave_group(&b));
    {
        // Rellenar hasta el tope sin pasar por el límite por dueño (como lo haría la API).
        let mut st = s.state();
        while st.groups.len() < GROUPS_MAX {
            let id = st.next_group;
            st.next_group += 1;
            let group = Group {
                name: format!("relleno {id}"),
                password: None,
                owner: None,
                persistent: true,
                saved_members: HashSet::new(),
            };
            st.groups.insert(id, group);
        }
    }
    assert_eq!(s.create_group(&b, "ya no cabe", None), Err(GroupError::TooMany));
}

/// Minecraft manda a veces posiciones con NaN. Guardarlas dejaba a ese jugador sin oír
/// ni ser oído, y sin un solo aviso.
#[test]
fn a_position_that_is_not_a_number_is_ignored() {
    let s = server();
    let a = [1; 16];
    s.add_player(a);
    s.update_player(&a, [10.0, 64.0, -5.0], 1);
    for bad in [[f64::NAN, 64.0, 0.0], [0.0, f64::INFINITY, 0.0], [0.0, 64.0, f64::NEG_INFINITY]] {
        s.update_player(&a, bad, 1);
        let st = s.state();
        let p = &st.players[&a];
        assert_eq!(p.pos, [10.0, 64.0, -5.0], "se queda donde estaba con {bad:?}");
        assert_eq!(p.cell, Cell::of([10.0, 64.0, -5.0], 1));
    }
    // Y una buena sí se aplica.
    s.update_player(&a, [20.0, 65.0, 1.0], 1);
    assert_eq!(s.state().players[&a].pos, [20.0, 65.0, 1.0]);
}

/// Al limpiar la tabla de IP ruidosas solo se olvidan las que ya han pagado su castigo:
/// antes se vaciaba entera, así que una ráfaga desde muchas direcciones borraba también
/// el bloqueo de quien estaba abusando.
#[test]
fn clearing_the_noisy_ip_table_keeps_the_ones_still_blocked() {
    let s = server();
    let abuser = "203.0.113.7:4000".parse().unwrap();
    for _ in 0..(BAD_PACKETS_BURST as usize + 5) {
        let _ = s.shared.handle(b"basura", Source::Udp(abuser), &mut Vec::new(), &mut Vec::new());
    }
    assert!(s.state().bad_ips.get_mut(&abuser.ip()).unwrap().is_empty(), "se le acabó el crédito");

    // Muchas direcciones distintas, cada una con un solo paquete malo: les sobra crédito.
    for n in 0..=BAD_IPS_MAX {
        let addr = format!("198.51.100.{}:{}", n % 256, 5000 + n / 256).parse().unwrap();
        let _ = s.shared.handle(b"basura", Source::Udp(addr), &mut Vec::new(), &mut Vec::new());
    }
    let st = s.state();
    assert!(st.bad_ips.len() <= BAD_IPS_MAX + 1, "la tabla no crece sin fin: {}", st.bad_ips.len());
    assert!(st.bad_ips.contains_key(&abuser.ip()), "el que abusa sigue castigado");
}
