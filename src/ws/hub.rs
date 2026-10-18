use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

use tokio::sync::mpsc;
use uuid::Uuid;

const CLIENT_BUFFER: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Customer,
    Staff,
}

pub type ConnId = Uuid;
type Tx = mpsc::Sender<String>;

#[derive(Default)]
struct Registry {
    by_user: HashMap<Uuid, HashMap<ConnId, Tx>>,
    staff: HashMap<ConnId, (Uuid, Tx)>, // (user_id, sender)
}

#[derive(Clone, Default)]
pub struct Hub {
    inner: Arc<RwLock<Registry>>,
}

impl Hub {
    pub fn register(&self, user_id: Uuid, role: Role) -> (ConnId, mpsc::Receiver<String>) {
        let (tx, rx) = mpsc::channel(CLIENT_BUFFER);
        let conn_id = Uuid::new_v4();
        let mut reg = self.inner.write().unwrap();
        reg.by_user
            .entry(user_id)
            .or_default()
            .insert(conn_id, tx.clone());
        if role == Role::Staff {
            reg.staff.insert(conn_id, (user_id, tx));
        }
        (conn_id, rx)
    }

    pub fn unregister(&self, user_id: Uuid, conn_id: ConnId) {
        let mut reg = self.inner.write().unwrap();
        remove_conn(&mut reg, user_id, conn_id);
    }

    /// Returns how many connections accepted the message.
    pub fn send_to_user(&self, user_id: Uuid, msg: &str) -> usize {
        let (delivered, dead) = {
            let reg = self.inner.read().unwrap();
            match reg.by_user.get(&user_id) {
                Some(conns) => push(conns.iter().map(|(c, tx)| (user_id, *c, tx)), msg),
                None => (0, Vec::new()),
            }
        };
        self.reap(dead);
        delivered
    }

    pub fn broadcast_staff(&self, msg: &str) -> usize {
        let (delivered, dead) = {
            let reg = self.inner.read().unwrap();
            push(reg.staff.iter().map(|(c, (u, tx))| (*u, *c, tx)), msg)
        };
        self.reap(dead);
        delivered
    }

    fn reap(&self, dead: Vec<(Uuid, ConnId)>) {
        if dead.is_empty() {
            return;
        }
        let mut reg = self.inner.write().unwrap();
        for (user_id, conn_id) in dead {
            tracing::warn!(%conn_id, "dropping slow or closed websocket client");
            remove_conn(&mut reg, user_id, conn_id);
        }
    }
}

fn remove_conn(reg: &mut Registry, user_id: Uuid, conn_id: ConnId) {
    if let Some(conns) = reg.by_user.get_mut(&user_id) {
        conns.remove(&conn_id);
        if conns.is_empty() {
            reg.by_user.remove(&user_id);
        }
    }
    reg.staff.remove(&conn_id);
}

/// try_send never blocks: a full buffer means the client is too slow, so it's dropped.
fn push<'a>(
    targets: impl Iterator<Item = (Uuid, ConnId, &'a Tx)>,
    msg: &str,
) -> (usize, Vec<(Uuid, ConnId)>) {
    let mut delivered = 0;
    let mut dead = Vec::new();
    for (user_id, conn_id, tx) in targets {
        match tx.try_send(msg.to_owned()) {
            Ok(()) => delivered += 1,
            Err(_) => dead.push((user_id, conn_id)),
        }
    }
    (delivered, dead)
}
