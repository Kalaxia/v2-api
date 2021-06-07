Kalaxia API
===========

Kalaxia is a free multiplayer strategy-game taking place in space.

The client is powered by Godot Engine, and the server-side is powered by Rust.

Setup
-----

**Prerequisites**

* Rust <= 1.52.1
* Cargo <= 1.52

**Instructions**

First things first, you can fork or directly clone the repository :

```
git clone git@github.com:Kalaxia/v2-api.git kalaxia-api
```

To run the game, simply use ``cargo run``.

With Docker
-----------

If you want to get the exact same environment as the production to check if your work is compatible, you can use Docker and Docker Compose.

Before that you must copy the environment configuration files :

```
cp .dist.env .env
cp kalaxia.dist.env kalaxia.env
```

The game server enables SSL by default. If you have forged a local certificate for test purpose, you can drop the cert and the key in ``./var/ssl`` and then set the file names in ``kalaxia.env`` (the path must start with ``/var/ssl`` which is the Docker remote folder bound to ``./var/ssl``).

**To disable the default SSL feature**, go to ``.env`` file and remove the ``ssl-secure`` feature. You shall obtain the following file:

```
FEATURES=
```

At this moment you can start your Docker container :

```
docker-compose up -d
```

(At the current time, the Docker build is quite long and has a high CPU-consumption. The team will work soon to simplify that but for now Docker building is not the simpler way to run the game server)

Documentation
-------------

The API documentation can be found [here](https://app.swaggerhub.com/apis/Kalaxia/kalaxia-api/1.0.0).

The Websocket endpoints are documented [here](/doc/websockets.md)

Contribute
----------

Kalaxia is a free open-source project. You can join the team anytime and reach us via [Discord](https://discordapp.com/invite/mDTms5c).

You are free to improve the game code by opening a pull request. This concerns only technical improvements.

To develop new features and feedbacks, first consult with the team in order to stay organized.

The gitflow is described in the documentation.
