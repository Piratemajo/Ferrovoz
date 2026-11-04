//! Envío de muchos paquetes UDP de golpe.
//!
//! En Linux, `sendmmsg`: un syscall para hasta [`BATCH`] paquetes (el coste está
//! sobre todo en el syscall, no en los bytes). En el resto, un `send_to` por paquete.

use std::net::{SocketAddr, UdpSocket};

/// Paquetes por syscall en Linux.
pub const BATCH: usize = 64;

/// Envía todos los paquetes; devuelve cuántos salieron.
pub fn send_all(socket: &UdpSocket, packets: &[(SocketAddr, Vec<u8>)]) -> usize {
    #[cfg(target_os = "linux")]
    {
        linux::send_all(socket, packets)
    }
    #[cfg(not(target_os = "linux"))]
    {
        packets.iter().filter(|(addr, pkt)| socket.send_to(pkt, addr).is_ok()).count()
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::BATCH;
    use std::net::{SocketAddr, UdpSocket};
    use std::os::fd::AsRawFd;

    /// Dirección de destino en el formato de C, para un `msghdr`.
    fn sockaddr(addr: &SocketAddr) -> (libc::sockaddr_storage, libc::socklen_t) {
        // SAFETY: sockaddr_storage es una estructura C de bytes; todo ceros es válido.
        let mut storage: libc::sockaddr_storage = unsafe { std::mem::zeroed() };
        let len = match addr {
            SocketAddr::V4(a) => {
                let sin = libc::sockaddr_in {
                    sin_family: libc::AF_INET as libc::sa_family_t,
                    sin_port: a.port().to_be(),
                    sin_addr: libc::in_addr { s_addr: u32::from_ne_bytes(a.ip().octets()) },
                    sin_zero: [0; 8],
                };
                // SAFETY: sockaddr_storage tiene tamaño y alineación para cualquier sockaddr.
                unsafe { std::ptr::write(&mut storage as *mut _ as *mut libc::sockaddr_in, sin) };
                std::mem::size_of::<libc::sockaddr_in>()
            }
            SocketAddr::V6(a) => {
                let sin6 = libc::sockaddr_in6 {
                    sin6_family: libc::AF_INET6 as libc::sa_family_t,
                    sin6_port: a.port().to_be(),
                    sin6_flowinfo: a.flowinfo(),
                    sin6_addr: libc::in6_addr { s6_addr: a.ip().octets() },
                    sin6_scope_id: a.scope_id(),
                };
                // SAFETY: igual que arriba.
                unsafe { std::ptr::write(&mut storage as *mut _ as *mut libc::sockaddr_in6, sin6) };
                std::mem::size_of::<libc::sockaddr_in6>()
            }
        };
        (storage, len as libc::socklen_t)
    }

    pub fn send_all(socket: &UdpSocket, packets: &[(SocketAddr, Vec<u8>)]) -> usize {
        if packets.len() <= 1 {
            return packets.iter().filter(|(addr, pkt)| socket.send_to(pkt, addr).is_ok()).count();
        }
        let fd = socket.as_raw_fd();
        let mut sent = 0;
        for chunk in packets.chunks(BATCH) {
            let mut addrs: Vec<(libc::sockaddr_storage, libc::socklen_t)> = chunk.iter().map(|(a, _)| sockaddr(a)).collect();
            let mut iovs: Vec<libc::iovec> = chunk
                .iter()
                .map(|(_, p)| libc::iovec { iov_base: p.as_ptr() as *mut libc::c_void, iov_len: p.len() })
                .collect();
            let mut msgs: Vec<libc::mmsghdr> = (0..chunk.len())
                .map(|i| {
                    // SAFETY: mmsghdr es una estructura C; todo ceros es un valor válido.
                    let mut m: libc::mmsghdr = unsafe { std::mem::zeroed() };
                    m.msg_hdr.msg_name = &mut addrs[i].0 as *mut _ as *mut libc::c_void;
                    m.msg_hdr.msg_namelen = addrs[i].1;
                    m.msg_hdr.msg_iov = &mut iovs[i];
                    m.msg_hdr.msg_iovlen = 1;
                    m
                })
                .collect();
            let mut start = 0;
            while start < msgs.len() {
                // SAFETY: msgs, iovs y addrs viven hasta el final del bucle y los punteros
                // apuntan a ellos; los paquetes (iov_base) solo se leen.
                let n = unsafe {
                    libc::sendmmsg(fd, msgs[start..].as_mut_ptr(), (msgs.len() - start) as libc::c_uint, 0)
                };
                if n > 0 {
                    sent += n as usize;
                    start += n as usize;
                } else {
                    let err = std::io::Error::last_os_error();
                    if err.kind() == std::io::ErrorKind::Interrupted {
                        continue;
                    }
                    // Falla el primero del lote (p. ej. destino inalcanzable): se salta.
                    start += 1;
                }
            }
        }
        sent
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn every_packet_arrives_to_its_own_destination() {
        let tx = UdpSocket::bind("127.0.0.1:0").unwrap();
        let receivers: Vec<UdpSocket> = (0..5).map(|_| UdpSocket::bind("127.0.0.1:0").unwrap()).collect();
        for r in &receivers {
            r.set_read_timeout(Some(Duration::from_millis(500))).unwrap();
        }
        // 150 paquetes (más de dos lotes) a 5 destinos.
        let packets: Vec<(SocketAddr, Vec<u8>)> = (0..150u32)
            .map(|i| (receivers[i as usize % 5].local_addr().unwrap(), i.to_le_bytes().repeat(10)))
            .collect();
        assert_eq!(send_all(&tx, &packets), 150);
        let mut got = vec![Vec::new(); 5];
        let mut buf = [0u8; 100];
        for (k, r) in receivers.iter().enumerate() {
            for _ in 0..30 {
                let n = r.recv(&mut buf).unwrap();
                assert_eq!(n, 40);
                got[k].push(u32::from_le_bytes(buf[..4].try_into().unwrap()));
            }
        }
        for (k, ids) in got.iter().enumerate() {
            let expected: Vec<u32> = (0..150u32).filter(|i| *i as usize % 5 == k).collect();
            assert_eq!(ids, &expected, "cada destino recibe los suyos, en orden");
        }
    }

    #[test]
    fn ipv6_destinations_work_too() {
        let (Ok(tx), Ok(rx)) = (UdpSocket::bind("[::1]:0"), UdpSocket::bind("[::1]:0")) else {
            return; // sin IPv6 en esta máquina
        };
        rx.set_read_timeout(Some(Duration::from_millis(500))).unwrap();
        let to = rx.local_addr().unwrap();
        let packets: Vec<(SocketAddr, Vec<u8>)> = (0..3u8).map(|i| (to, vec![i; 8])).collect();
        assert_eq!(send_all(&tx, &packets), 3);
        let mut buf = [0u8; 16];
        for i in 0..3u8 {
            assert_eq!(rx.recv(&mut buf).unwrap(), 8);
            assert_eq!(buf[0], i);
        }
    }
}
