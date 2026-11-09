//! Pruebas del relé (lo que no necesita red de verdad; lo demás está en tests/).

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
