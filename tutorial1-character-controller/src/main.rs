use fyrox::{
    core::{
        algebra::{UnitQuaternion, Vector3},
        pool::Handle,
    },
    engine::{resource_manager::ResourceManager, Engine, EngineInitParams, SerializationContext},
    event::{DeviceEvent, ElementState, Event, VirtualKeyCode, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    resource::texture::TextureWrapMode,
    scene::{
        base::BaseBuilder,
        camera::{CameraBuilder, SkyBox, SkyBoxBuilder},
        collider::{ColliderBuilder, ColliderShape},
        node::Node,
        rigidbody::RigidBodyBuilder,
        transform::TransformBuilder,
        Scene,
    },
    window::WindowBuilder,
};
use std::{sync::Arc, time};

// Our game logic will be updated at 60 Hz rate.
const TIMESTEP: f32 = 1.0 / 60.0;

#[derive(Default)]
struct InputController {
    move_forward: bool,
    move_backward: bool,
    move_left: bool,
    move_right: bool,
    pitch: f32,
    yaw: f32,
}

struct Player {
    camera: Handle<Node>,
    rigid_body: Handle<Node>,
    controller: InputController,
}

async fn create_skybox(resource_manager: ResourceManager) -> SkyBox {
    // Load skybox textures in parallel.
    let (front, back, left, right, top, bottom) = fyrox::core::futures::join!(
        resource_manager.request_texture("data/textures/skybox/front.jpg"),
        resource_manager.request_texture("data/textures/skybox/back.jpg"),
        resource_manager.request_texture("data/textures/skybox/left.jpg"),
        resource_manager.request_texture("data/textures/skybox/right.jpg"),
        resource_manager.request_texture("data/textures/skybox/up.jpg"),
        resource_manager.request_texture("data/textures/skybox/down.jpg")
    );

    // Unwrap everything.
    let skybox = SkyBoxBuilder {
        front: Some(front.unwrap()),
        back: Some(back.unwrap()),
        left: Some(left.unwrap()),
        right: Some(right.unwrap()),
        top: Some(top.unwrap()),
        bottom: Some(bottom.unwrap()),
    }
    .build()
    .unwrap();

    // Set S and T coordinate wrap mode, ClampToEdge will remove any possible seams on edges
    // of the skybox.
    let skybox_texture = skybox.cubemap().unwrap();
    let mut data = skybox_texture.data_ref();
    data.set_s_wrap_mode(TextureWrapMode::ClampToEdge);
    data.set_t_wrap_mode(TextureWrapMode::ClampToEdge);

    skybox
}

impl Player {
    async fn new(scene: &mut Scene, resource_manager: ResourceManager) -> Self {
        // Create rigid body with a camera, move it a bit up to "emulate" head.
        let camera;
        let rigid_body_handle = RigidBodyBuilder::new(
            BaseBuilder::new()
                .with_local_transform(
                    TransformBuilder::new()
                        // Offset player a bit.
                        .with_local_position(Vector3::new(0.0, 1.0, -1.0))
                        .build(),
                )
                .with_children(&[
                    {
                        camera = CameraBuilder::new(
                            BaseBuilder::new().with_local_transform(
                                TransformBuilder::new()
                                    .with_local_position(Vector3::new(0.0, 0.25, 0.0))
                                    .build(),
                            ),
                        )
                        .with_skybox(create_skybox(resource_manager).await)
                        .build(&mut scene.graph);
                        camera
                    },
                    // Add capsule collider for the rigid body.
                    ColliderBuilder::new(BaseBuilder::new())
                        .with_shape(ColliderShape::capsule_y(0.25, 0.2))
                        .build(&mut scene.graph),
                ]),
        )
        // We don't want the player to tilt.
        .with_locked_rotations(true)
        // We don't want the rigid body to sleep (be excluded from simulation)
        .with_can_sleep(false)
        .build(&mut scene.graph);

        Self {
            camera,
            rigid_body: rigid_body_handle,
            controller: Default::default(),
        }
    }

    fn update(&mut self, scene: &mut Scene) {
        // Set pitch for the camera. These lines responsible for up-down camera rotation.
        scene.graph[self.camera].local_transform_mut().set_rotation(
            UnitQuaternion::from_axis_angle(&Vector3::x_axis(), self.controller.pitch.to_radians()),
        );

        // Borrow rigid body node.
        let body = scene.graph[self.rigid_body].as_rigid_body_mut();

        // Keep only vertical velocity, and drop horizontal.
        let mut velocity = Vector3::new(0.0, body.lin_vel().y, 0.0);

        // Change the velocity depending on the keys pressed.
        if self.controller.move_forward {
            // If we moving forward then add "look" vector of the body.
            velocity += body.look_vector();
        }
        if self.controller.move_backward {
            // If we moving backward then subtract "look" vector of the body.
            velocity -= body.look_vector();
        }
        if self.controller.move_left {
            // If we moving left then add "side" vector of the body.
            velocity += body.side_vector();
        }
        if self.controller.move_right {
            // If we moving right then subtract "side" vector of the body.
            velocity -= body.side_vector();
        }

        // Finally new linear velocity.
        body.set_lin_vel(velocity);

        // Change the rotation of the rigid body according to current yaw. These lines responsible for
        // left-right rotation.
        body.local_transform_mut()
            .set_rotation(UnitQuaternion::from_axis_angle(
                &Vector3::y_axis(),
                self.controller.yaw.to_radians(),
            ));
    }

    fn process_input_event(&mut self, event: &Event<()>) {
        match event {
            Event::WindowEvent { event, .. } => {
                if let WindowEvent::KeyboardInput { input, .. } = event {
                    if let Some(key_code) = input.virtual_keycode {
                        match key_code {
                            VirtualKeyCode::W => {
                                self.controller.move_forward = input.state == ElementState::Pressed;
                            }
                            VirtualKeyCode::S => {
                                self.controller.move_backward =
                                    input.state == ElementState::Pressed;
                            }
                            VirtualKeyCode::A => {
                                self.controller.move_left = input.state == ElementState::Pressed;
                            }
                            VirtualKeyCode::D => {
                                self.controller.move_right = input.state == ElementState::Pressed;
                            }
                            _ => (),
                        }
                    }
                }
            }
            Event::DeviceEvent { event, .. } => {
                if let DeviceEvent::MouseMotion { delta } = event {
                    self.controller.yaw -= delta.0 as f32;

                    self.controller.pitch =
                        (self.controller.pitch + delta.1 as f32).clamp(-90.0, 90.0);
                }
            }
            _ => (),
        }
    }
}

struct Game {
    scene: Handle<Scene>,
    player: Player,
}

impl Game {
    pub async fn new(engine: &mut Engine) -> Self {
        let mut scene = Scene::new();

        // Load a scene resource and create its instance.
        engine
            .resource_manager
            .request_model("data/models/scene.rgs")
            .await
            .unwrap()
            .instantiate_geometry(&mut scene);

        Self {
            player: Player::new(&mut scene, engine.resource_manager.clone()).await,
            scene: engine.scenes.add(scene),
        }
    }

    pub fn update(&mut self, engine: &mut Engine) {
        self.player.update(&mut engine.scenes[self.scene]);
    }
}

fn main() {
    // Configure main window first.
    let window_builder = WindowBuilder::new().with_title("3D Shooter Tutorial");
    // Create event loop that will be used to "listen" events from the OS.
    let event_loop = EventLoop::new();

    // Finally create an instance of the engine.
    let serialization_context = Arc::new(SerializationContext::new());
    let mut engine = Engine::new(EngineInitParams {
        window_builder,
        resource_manager: ResourceManager::new(serialization_context.clone()),
        serialization_context,
        events_loop: &event_loop,
        vsync: false,
    })
    .unwrap();

    // Initialize game instance.
    let mut game = fyrox::core::futures::executor::block_on(Game::new(&mut engine));

    // Run the event loop of the main window. which will respond to OS and window events and update
    // engine's state accordingly. Engine lets you to decide which event should be handled,
    // this is minimal working example if how it should be.
    let clock = time::Instant::now();
    let mut elapsed_time = 0.0;
    event_loop.run(move |event, _, control_flow| {
        game.player.process_input_event(&event);

        match event {
            Event::MainEventsCleared => {
                // This main game loop - it has fixed time step which means that game
                // code will run at fixed speed even if renderer can't give you desired
                // 60 fps.
                let mut dt = clock.elapsed().as_secs_f32() - elapsed_time;
                while dt >= TIMESTEP {
                    dt -= TIMESTEP;
                    elapsed_time += TIMESTEP;

                    // Run our game's logic.
                    game.update(&mut engine);

                    // Update engine each frame.
                    engine.update(TIMESTEP, control_flow);
                }

                // Rendering must be explicitly requested and handled after RedrawRequested event is received.
                engine.get_window().request_redraw();
            }
            Event::RedrawRequested(_) => {
                // Render at max speed - it is not tied to the game code.
                engine.render().unwrap();
            }
            Event::WindowEvent { event, .. } => match event {
                WindowEvent::CloseRequested => *control_flow = ControlFlow::Exit,
                WindowEvent::KeyboardInput { input, .. } => {
                    // Exit game by hitting Escape.
                    if let Some(VirtualKeyCode::Escape) = input.virtual_keycode {
                        *control_flow = ControlFlow::Exit
                    }
                }
                WindowEvent::Resized(size) => {
                    // It is very important to handle Resized event from window, because
                    // renderer knows nothing about window size - it must be notified
                    // directly when window size has changed.
                    engine.set_frame_size(size.into()).unwrap();
                }
                _ => (),
            },
            _ => *control_flow = ControlFlow::Poll,
        }
    });
}
