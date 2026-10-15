//! Mute in SPA params, read and written the way WirePlumber's mixer-api
//! does (wireplumber 0.5.18 modules/module-mixer-api.c):
//! - collect_node_info (:176): the node's device (`device.id`) has a Route
//!   whose `device` is the node's `card.profile.device` and whose `props`
//!   hold both `mute` and `channelVolumes` (node_info_fill, :132): that
//!   route is where the mute lives. Otherwise the node's own Props.
//! - wp_mixer_api_set_volume (:397): a Props object holding only what
//!   changes, set on the device as a Route { index, device, props, save:
//!   true } (:509), or else on the node as Props (:522).

use std::io::Cursor;

use pipewire::spa::pod::deserialize::PodDeserializer;
use pipewire::spa::pod::serialize::PodSerializer;
use pipewire::spa::pod::{Object, Property, PropertyFlags, Value};
use pipewire::spa::sys as spa;

fn object(pod: &[u8]) -> Option<Object> {
    match PodDeserializer::deserialize_any_from(pod) {
        Ok((_, Value::Object(object))) => Some(object),
        _ => None,
    }
}

fn get(object: &Object, key: u32) -> Option<&Value> {
    object
        .properties
        .iter()
        .find_map(|property| (property.key == key).then_some(&property.value))
}

/// node_info_fill: a mute, and channelVolumes beside it.
fn mute_of(props: &Object) -> Option<bool> {
    get(props, spa::SPA_PROP_channelVolumes)?;
    match get(props, spa::SPA_PROP_mute)? {
        Value::Bool(muted) => Some(*muted),
        _ => None,
    }
}

/// A node's Props param: its mute, if the mixer would use it.
pub fn props_mute(pod: &[u8]) -> Option<bool> {
    mute_of(&object(pod)?)
}

/// A device's Route param, as far as the mixer cares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Route {
    pub index: i32,
    pub device: i32,
    /// None if its props lack mute or channelVolumes: then the mixer skips it.
    pub mute: Option<bool>,
}

pub fn route(pod: &[u8]) -> Option<Route> {
    let route = object(pod)?;
    let int = |key| match get(&route, key) {
        Some(Value::Int(value)) => Some(*value),
        _ => None,
    };
    let mute = match get(&route, spa::SPA_PARAM_ROUTE_props) {
        Some(Value::Object(props)) => mute_of(props),
        _ => None,
    };
    Some(Route {
        index: int(spa::SPA_PARAM_ROUTE_index)?,
        device: int(spa::SPA_PARAM_ROUTE_device)?,
        mute,
    })
}

fn property(key: u32, value: Value) -> Property {
    Property {
        key,
        flags: PropertyFlags::empty(),
        value,
    }
}

fn mute_props(muted: bool) -> Object {
    Object {
        type_: spa::SPA_TYPE_OBJECT_Props,
        id: spa::SPA_PARAM_Props,
        properties: vec![property(spa::SPA_PROP_mute, Value::Bool(muted))],
    }
}

fn serialize(object: Object) -> Vec<u8> {
    PodSerializer::serialize(Cursor::new(Vec::new()), &Value::Object(object))
        .expect("an object of fixed-size fields serializes")
        .0
        .into_inner()
}

/// The node Props that set the mute.
pub fn props_pod(muted: bool) -> Vec<u8> {
    serialize(mute_props(muted))
}

/// The device Route that sets the mute of `route`.
pub fn route_pod(index: i32, device: i32, muted: bool) -> Vec<u8> {
    serialize(Object {
        type_: spa::SPA_TYPE_OBJECT_ParamRoute,
        id: spa::SPA_PARAM_Route,
        properties: vec![
            property(spa::SPA_PARAM_ROUTE_index, Value::Int(index)),
            property(spa::SPA_PARAM_ROUTE_device, Value::Int(device)),
            property(spa::SPA_PARAM_ROUTE_props, Value::Object(mute_props(muted))),
            property(spa::SPA_PARAM_ROUTE_save, Value::Bool(true)),
        ],
    })
}

/// Where the mixer reads and writes a node's mute.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Via {
    /// Route `index` of profile device `device`, on PipeWire device `on`.
    Route {
        on: u32,
        index: i32,
        device: i32,
    },
    Props,
}

/// collect_node_info: the node's route if it qualifies, else its Props.
pub fn mixer(
    node_device: Option<u32>,
    profile_device: Option<i32>,
    routes: impl Fn(u32) -> Option<Vec<Route>>,
    props_mute: Option<bool>,
) -> Option<(Via, bool)> {
    if let (Some(on), Some(device)) = (node_device, profile_device)
        && let Some(routes) = routes(on)
        && let Some(route) = routes
            .iter()
            .find(|route| route.device == device && route.mute.is_some())
    {
        let via = Via::Route {
            on,
            index: route.index,
            device,
        };
        return route.mute.map(|muted| (via, muted));
    }
    props_mute.map(|muted| (Via::Props, muted))
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use pipewire::spa::pod::ValueArray;

    /// A node's Props, as audioconvert reports them.
    pub fn node_props(muted: bool) -> Vec<u8> {
        serialize(Object {
            type_: spa::SPA_TYPE_OBJECT_Props,
            id: spa::SPA_PARAM_Props,
            properties: vec![
                property(spa::SPA_PROP_volume, Value::Float(1.0)),
                property(spa::SPA_PROP_mute, Value::Bool(muted)),
                property(
                    spa::SPA_PROP_channelVolumes,
                    Value::ValueArray(ValueArray::Float(vec![1.0, 1.0])),
                ),
            ],
        })
    }

    /// A device's Route, as ACP reports one.
    pub fn device_route(index: i32, device: i32, muted: Option<bool>) -> Vec<u8> {
        let mut props = vec![property(
            spa::SPA_PROP_channelVolumes,
            Value::ValueArray(ValueArray::Float(vec![0.5, 0.5])),
        )];
        if let Some(muted) = muted {
            props.push(property(spa::SPA_PROP_mute, Value::Bool(muted)));
        }
        serialize(Object {
            type_: spa::SPA_TYPE_OBJECT_ParamRoute,
            id: spa::SPA_PARAM_Route,
            properties: vec![
                property(spa::SPA_PARAM_ROUTE_index, Value::Int(index)),
                property(
                    spa::SPA_PARAM_ROUTE_direction,
                    Value::Id(pipewire::spa::utils::Id(0)),
                ),
                property(spa::SPA_PARAM_ROUTE_device, Value::Int(device)),
                property(
                    spa::SPA_PARAM_ROUTE_name,
                    Value::String("analog-input-mic".into()),
                ),
                property(
                    spa::SPA_PARAM_ROUTE_props,
                    Value::Object(Object {
                        type_: spa::SPA_TYPE_OBJECT_Props,
                        id: spa::SPA_PARAM_Route,
                        properties: props,
                    }),
                ),
            ],
        })
    }

    #[test]
    fn props_count_only_with_mute_and_channel_volumes() {
        assert_eq!(props_mute(&node_props(true)), Some(true));
        assert_eq!(props_mute(&node_props(false)), Some(false));
        // the set pod itself has no channelVolumes: not something to read
        assert_eq!(props_mute(&props_pod(true)), None);
        assert_eq!(props_mute(b"junk"), None);
    }

    #[test]
    fn routes_parse_index_device_and_mute() {
        assert_eq!(
            route(&device_route(3, 7, Some(true))),
            Some(Route {
                index: 3,
                device: 7,
                mute: Some(true)
            })
        );
        assert_eq!(
            route(&device_route(3, 7, None)).map(|route| route.mute),
            Some(None)
        );
    }

    #[test]
    fn the_set_pods_carry_what_the_mixer_sets() {
        let Some(Value::Object(set)) =
            PodDeserializer::deserialize_any_from(&route_pod(3, 7, true))
                .ok()
                .map(|(_, value)| value)
        else {
            panic!("not an object");
        };
        assert_eq!(
            (set.type_, set.id),
            (spa::SPA_TYPE_OBJECT_ParamRoute, spa::SPA_PARAM_Route)
        );
        assert_eq!(get(&set, spa::SPA_PARAM_ROUTE_index), Some(&Value::Int(3)));
        assert_eq!(get(&set, spa::SPA_PARAM_ROUTE_device), Some(&Value::Int(7)));
        assert_eq!(
            get(&set, spa::SPA_PARAM_ROUTE_save),
            Some(&Value::Bool(true))
        );
        let Some(Value::Object(props)) = get(&set, spa::SPA_PARAM_ROUTE_props) else {
            panic!("no props");
        };
        assert_eq!(props.properties.len(), 1, "only the mute changes");
        assert_eq!(get(props, spa::SPA_PROP_mute), Some(&Value::Bool(true)));
        let props = object(&props_pod(false)).unwrap();
        assert_eq!(
            (props.type_, props.id),
            (spa::SPA_TYPE_OBJECT_Props, spa::SPA_PARAM_Props)
        );
        assert_eq!(
            props.properties,
            vec![property(spa::SPA_PROP_mute, Value::Bool(false))]
        );
    }

    #[test]
    fn the_mixer_prefers_the_matching_route_and_falls_back_to_props() {
        let routes = |on: u32| {
            (on == 40).then(|| {
                vec![
                    Route {
                        index: 0,
                        device: 2,
                        mute: Some(false),
                    }, // the speakers
                    Route {
                        index: 1,
                        device: 5,
                        mute: Some(true),
                    }, // the mic
                    Route {
                        index: 2,
                        device: 6,
                        mute: None,
                    },
                ]
            })
        };
        let via = |on, index, device| Via::Route { on, index, device };
        assert_eq!(
            mixer(Some(40), Some(5), routes, Some(false)),
            Some((via(40, 1, 5), true))
        );
        // no route for its profile device, or one without a mute: Props
        assert_eq!(
            mixer(Some(40), Some(9), routes, Some(false)),
            Some((Via::Props, false))
        );
        assert_eq!(
            mixer(Some(40), Some(6), routes, Some(false)),
            Some((Via::Props, false))
        );
        // a node of no device (a virtual source), or one not bound
        assert_eq!(
            mixer(None, Some(5), routes, Some(true)),
            Some((Via::Props, true))
        );
        assert_eq!(mixer(Some(41), Some(5), routes, None), None);
    }
}
