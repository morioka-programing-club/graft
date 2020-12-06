use activitystreams::object::properties::{ObjectProperties, ApObjectProperties};
use serde::{Serialize, Deserialize};
use activitystreams::actor::{self, Actor as ActorTrait, properties::ApActorProperties};

#[derive(Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Actor {
	Person(actor::Person),
	Group(actor::Group),
	Organization(actor::Organization),
	Application(actor::Application),
	Service(actor::Service)
}

impl ActorTrait for Actor {}

impl AsRef<ObjectProperties> for Actor {

}

impl AsRef<ApObjectProperties> for Actor {

}

impl AsRef<ApActorProperties> for Actor {

}

impl AsMut<ObjectProperties> for Actor {

}

impl AsMut<ApObjectProperties> for Actor {

}

impl AsMut<ApActorProperties> for Actor {

}