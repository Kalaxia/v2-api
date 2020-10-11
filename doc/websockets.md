Websocket endpoints
===================

BuildingConstructed
-------------------
* **Description:** A new building has finished its construction
* **Level:** Faction
```json
{
    "id": "uuid",
    "system": "uuid",
    "kind": "mine",
    "status": "operational",
    "created_at": 152325465415,
    "built_at": 152325464123
}
```
BattleEnded
-------------------
* **Description:** System defenders have repelled an attack
* **Level:** Game
```json
{
    "system": {
        "id": "uuid",
        "game": "uuid",
        "player": "uuid",
        "kind": "BaseSystem",
        "coordinates": {
            "x": 15.75,
            "y": 10.2354
        },
        "unreachable": false
    },
    "fleets": {
        "fleet_uuid": {
            "id": "uuid",
            "system": "uuid",
            "destination_system": null,
            "destination_arrival_date": null,
            "player": "uuid",
            "ship_groups": [
                {
                    "id": "uuid",
                    "fleet": "uuid",
                    "kind": "fighter",
                    "quantity": 10
                }
            ]
        },
        "fleet_uuid": {
            "id": "uuid",
            "system": "uuid",
            "destination_system": "uuid",
            "destination_arrival_date": "uuid",
            "player": "uuid",
            "ship_groups": []
        }
    }
}
```
FactionPointsUpdated
-------------------
* **Description:** Victory points distribution
* **Level:** Game
```json
[
    {
        "faction": 1,
        "game": "uuid",
        "victory_points": 150
    },
    {
        "faction": 2,
        "game": "uuid",
        "victory_points": 120
    }
]
```
FleetCreated
-------------------
* **Description:** A new fleet has been created
* **Level:** Game
```json
{
    "id": "uuid",
    "system": "uuid",
    "destination_system": "uuid",
    "destination_arrival_date": 150331554212,
    "player": "uuid",
    "ship_groups": []
}
```
FleetArrived
-------------------
* **Description:** A fleet arrived at its destination without engaging enemy fleets
* **Level:** Game
```json
{
    "id": "uuid",
    "system": "uuid",
    "destination_system": null,
    "destination_arrival_date": null,
    "player": "uuid",
    "ship_groups": []
}
```
FleetSailed
-------------------
* **Description:** A fleet has begun a new journey
* **Level:** Game
```json
{
    "id": "uuid",
    "system": "uuid",
    "destination_system": "uuid",
    "destination_arrival_date": 150331554212,
    "player": "uuid",
    "ship_groups": []
}
```
FleetTransfer
-------------------
* **Description:** A player gave one of his fleets to an ally
* **Level:** Game
```json
{
    "donator_id": "uuid",
    "receiver_id": "uuid",
    "fleet": {
        "id": "uuid",
        "system": "uuid",
        "destination_system": "uuid",
        "destination_arrival_date": 150331554212,
        "player": "uuid",
        "ship_groups": []
    }
}
```
GameStarted
-------------------
* **Description:** Game is ready to be played
* **Level:** Game
```json
{}
```
LobbyCreated
-------------------
* **Description:** A new lobby has been created
* **Level:** Global
```json
{
    "id": "uuid",
    "owner": "uuid",
    "game_speed": "medium",
    "map_size": "medium"
}
```
LobbyOptionsUpdated
-------------------
* **Description:** Lobby owner updated game options
* **Level:** Lobby
```json
{
    "game_speed": "medium",
    "map_size": "medium"
}
```
LobbyOwnerUpdated
-------------------
* **Description:** Lobby owner has changed
* **Level:** Lobby
```json
{
    "id": "uuid",
    "game": null,
    "lobby": "uuid",
    "username": "Toto",
    "faction": 1,
    "wallet": 0,
    "is_ready": true,
    "is_connected": true
}
```
LobbyNameUpdated
-------------------
* **Description:** Lobby name has been updated
* **Level:** Global
```json
{
    "id": "uuid",
    "name": "Toto"
}
```
LobbyRemoved
-------------------
* **Description:** A lobby was removed
* **Level:** Global
```json
{
    "id": "uuid",
    "owner": "uuid",
    "game_speed": "medium",
    "map_size": "medium"
}
```
LobbyLaunched
-------------------
* **Description:** Lobby has been launched by its owner
* **Level:** Global
```json
{
    "id": "uuid",
    "owner": "uuid",
    "game_speed": "medium",
    "map_size": "medium"
}
```
PlayerConnected
-------------------
* **Description:** A new player connected to the server
* **Level:** Global
```json
{
    "id": "uuid",
    "game": null,
    "lobby": null,
    "username": "",
    "faction": null,
    "wallet": 0,
    "is_ready": false,
    "is_connected": true
}
```
PlayerJoined
-------------------
* **Description:** Player joined a lobby
* **Level:** Global
```json
{
    "id": "uuid",
    "game": null,
    "lobby": "uuid",
    "username": "",
    "faction": null,
    "wallet": 0,
    "is_ready": false,
    "is_connected": true
}
```
PlayerUpdate
-------------------
* **Description:** A player updated its informations
* **Level:** Lobby
```json
{
    "id": "uuid",
    "game": null,
    "lobby": "uuid",
    "username": "Toto",
    "faction": 1,
    "wallet": 0,
    "is_ready": false,
    "is_connected": true
}
```
PlayerMoneyTransfer
-------------------
* **Description:** An ally gave money to the notified player
* **Level:** Player
```json
{
    "player_id": "uuid",
    "amount": 500
}
```
PlayerLeft
-------------------
* **Description:** A player left the game or the lobby it was attached to
* **Level:** Game|Lobby
```json
{
    "pid": "uuid"
}
```
PlayerDisconnected
-------------------
* **Description:** A player has closed its connection to the server
* **Level:** Global
```json
{
    "id": "uuid",
    "game": "uuid",
    "lobby": null,
    "username": "Toto",
    "faction": 1,
    "wallet": 0,
    "is_ready": true,
    "is_connected": false
}
```
PlayerIncome
-------------------
* **Description:** Player wallet update
* **Level:** Player
```json
{
    "income": 1200
}
```
ShipQueueFinished
-------------------
* **Description:** Ship queue has delivered its ships
* **Level:** Player
```json
{
    "id": "uuid",
    "system": "uuid",
    "category": "fighter",
    "quantity": 10,
    "created_at": 15233564654,
    "started_at": 15235455452,
    "finished_at": 15234546411
}
```
SystemConquerred
-------------------
* **Description:** System has been conquerred and all defenders have been destroyed
* **Level:** Game
```json
{
    "system": {
        "id": "uuid",
        "game": "uuid",
        "player": "uuid",
        "kind": "BaseSystem",
        "coordinates": {
            "x": 15.75,
            "y": 10.2354
        },
        "unreachable": false
    },
    "fleet": {
        "id": "uuid",
        "system": "uuid",
        "destination_system": "uuid",
        "destination_arrival_date": 150331554212,
        "player": "uuid",
        "ship_groups": []
    }
}
```
SystemsCreated
-------------------
* **Description:** Galaxy map has been generated
* **Level:** Game
```json
{}
```
Victory
-------------------
* **Description:** A faction has emerged as a victor
* **Level:** Game
```json
{
    "victorious_faction": 1,
    "scores": [
        {
            "faction": 1,
            "game": "uuid",
            "victory_points": 150
        },
        {
            "faction": 2,
            "game": "uuid",
            "victory_points": 120
        }
    ]
}
```