use anyhow::{Context, Result, bail};
use pipewire as pw;
use pw::{
    node::{Node, NodeListener},
    spa,
    types::ObjectType,
};
use spa::pod::{
    Object, Pod, Property, Value, deserialize::PodDeserializer, serialize::PodSerializer,
};
use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque},
    io::Cursor,
    rc::Rc,
    time::{Duration, Instant},
};

enum Event {
    Cookie(u32),
    Added(u32, String, Node, NodeListener),
    Removed(u32),
    Mute(u32, bool),
    Error(u32, i32, String),
    Ready,
}

pub struct Sink {
    pub name: String,
    pub mute: Option<bool>,
    pub pending: Option<(bool, Instant)>,
    // Listeners must be dropped before the proxy they listen to.
    _listener: NodeListener,
    node: Node,
}

pub struct Audio {
    pub sinks: HashMap<u32, Sink>,
    pub cookie: Option<u32>,
    pub ready: bool,
    events: Rc<RefCell<VecDeque<Event>>>,
    _registry_listener: pw::registry::Listener,
    _core_listener: pw::core::Listener,
    _registry: pw::registry::RegistryRc,
    _core: pw::core::CoreRc,
    _context: pw::context::ContextRc,
    pub mainloop: pw::main_loop::MainLoopRc,
}

pub fn mute_from_pod(pod: &Pod) -> Option<bool> {
    let (_, Value::Object(obj)) = PodDeserializer::deserialize_any_from(pod.as_bytes()).ok()?
    else {
        return None;
    };
    obj.properties.iter().find_map(|p| match p.value {
        Value::Bool(value) if p.key == spa::sys::SPA_PROP_mute => Some(value),
        _ => None,
    })
}

pub fn mute_pod(value: bool) -> Result<Vec<u8>> {
    let pod = Value::Object(Object {
        type_: spa::sys::SPA_TYPE_OBJECT_Props,
        id: spa::sys::SPA_PARAM_Props,
        properties: vec![Property::new(spa::sys::SPA_PROP_mute, Value::Bool(value))],
    });
    let (bytes, _) = PodSerializer::serialize(Cursor::new(Vec::new()), &pod)
        .map_err(|e| anyhow::anyhow!("Serializing mute parameter: {e:?}"))?;
    Ok(bytes.into_inner())
}

impl Audio {
    pub fn connect() -> Result<Self> {
        pw::init();
        let mainloop = pw::main_loop::MainLoopRc::new(None)?;
        let context = pw::context::ContextRc::new(&mainloop, None)?;
        let core = context.connect_rc(Some(pw::properties::properties! {
            "application.name" => "Hushlight",
        }))?;
        let registry = core.get_registry_rc()?;
        let events = Rc::new(RefCell::new(VecDeque::new()));
        let info_q = events.clone();
        let error_q = events.clone();
        let done_q = events.clone();
        let seq = core.sync(0)?;
        let core_listener = core
            .add_listener_local()
            .info(move |info| info_q.borrow_mut().push_back(Event::Cookie(info.cookie())))
            .error(move |id, _, code, msg| {
                error_q
                    .borrow_mut()
                    .push_back(Event::Error(id, code, msg.into()))
            })
            .done(move |id, done| {
                if id == 0 && done == seq {
                    done_q.borrow_mut().push_back(Event::Ready);
                }
            })
            .register();
        let reg = registry.downgrade();
        let add_q = events.clone();
        let remove_q = events.clone();
        let registry_listener = registry
            .add_listener_local()
            .global(move |global| {
                if global.type_ != ObjectType::Node {
                    return;
                }
                let Some(props) = global.props.as_ref() else {
                    return;
                };
                if props.get("media.class") != Some("Audio/Sink") {
                    return;
                }
                let Some(name) = props.get("node.name") else {
                    return;
                };
                let Some(reg) = reg.upgrade() else {
                    return;
                };
                let node: Node = match reg.bind(global) {
                    Ok(node) => node,
                    Err(e) => {
                        add_q
                            .borrow_mut()
                            .push_back(Event::Error(global.id, 0, e.to_string()));
                        return;
                    }
                };
                let id = global.id;
                let param_q = add_q.clone();
                let listener = node
                    .add_listener_local()
                    .param(move |_, _, _, _, pod| {
                        if let Some(value) = pod.and_then(mute_from_pod) {
                            param_q.borrow_mut().push_back(Event::Mute(id, value));
                        }
                    })
                    .register();
                node.subscribe_params(&[spa::param::ParamType::Props]);
                add_q
                    .borrow_mut()
                    .push_back(Event::Added(id, name.into(), node, listener));
            })
            .global_remove(move |id| remove_q.borrow_mut().push_back(Event::Removed(id)))
            .register();
        Ok(Self {
            sinks: HashMap::new(),
            cookie: None,
            ready: false,
            events,
            _registry_listener: registry_listener,
            _core_listener: core_listener,
            _registry: registry,
            _core: core,
            _context: context,
            mainloop,
        })
    }

    pub fn dispatch(&mut self, timeout: Duration) -> Result<()> {
        self.mainloop
            .loop_()
            .iterate(pw::loop_::Timeout::Finite(timeout));
        while let Some(event) = self.events.borrow_mut().pop_front() {
            match event {
                Event::Cookie(cookie) => self.cookie = Some(cookie),
                Event::Added(id, name, node, listener) => {
                    self.sinks.insert(
                        id,
                        Sink {
                            name,
                            mute: None,
                            pending: None,
                            node,
                            _listener: listener,
                        },
                    );
                }
                Event::Removed(id) => {
                    self.sinks.remove(&id);
                }
                Event::Mute(id, value) => {
                    if let Some(sink) = self.sinks.get_mut(&id) {
                        sink.mute = Some(value);
                        if sink.pending.is_some_and(|(target, _)| target == value) {
                            sink.pending = None;
                        }
                    }
                }
                Event::Error(id, code, msg) if disconnected(id, code) => {
                    bail!("PipeWire disconnected: {msg}")
                }
                // A node can disappear between sending a command and its execution.
                // The core reports ENOENT for that race; the connection is still alive.
                Event::Error(_, code, _) if code == -libc::ENOENT => {}
                Event::Error(id, code, msg) => {
                    crate::log(&format!("PipeWire object {id} ({code}): {msg}"))
                }
                Event::Ready => self.ready = true,
            }
        }
        Ok(())
    }

    pub fn initialized(&self) -> bool {
        self.ready && self.cookie.is_some() && self.sinks.values().all(|sink| sink.mute.is_some())
    }

    pub fn set_mute(&mut self, id: u32, value: bool) -> Result<()> {
        let sink = self.sinks.get_mut(&id).context("Output disappeared")?;
        if let Some((target, sent)) = sink.pending {
            if target == value && sent.elapsed() < Duration::from_secs(1) {
                return Ok(());
            }
        } else if sink.mute == Some(value) {
            return Ok(());
        }
        let bytes = mute_pod(value)?;
        sink.node.set_param(
            spa::param::ParamType::Props,
            0,
            Pod::from_bytes(&bytes).context("Invalid mute pod")?,
        );
        sink.pending = Some((value, Instant::now()));
        Ok(())
    }
}

fn disconnected(id: u32, code: i32) -> bool {
    id == 0 && [-libc::EPIPE, -libc::ECONNRESET, -libc::ENOTCONN].contains(&code)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn removed_resource_is_not_a_server_disconnect() {
        assert!(!disconnected(0, -libc::ENOENT));
        assert!(disconnected(0, -libc::EPIPE));
        assert!(!disconnected(5, -libc::EPIPE));
    }
    #[test]
    fn mute_parameters_roundtrip_without_volume_fields() {
        for value in [false, true] {
            let bytes = mute_pod(value).unwrap();
            let pod = Pod::from_bytes(&bytes).unwrap();
            assert_eq!(mute_from_pod(pod), Some(value));
            let (_, Value::Object(obj)) = PodDeserializer::deserialize_any_from(&bytes).unwrap()
            else {
                panic!()
            };
            assert_eq!(obj.properties.len(), 1);
        }
    }
}
