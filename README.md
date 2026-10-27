# snrproj enviornment

## current structure
* **'cosmos-project/** - OpenC3 core
* **cosmos-project/plugins/openc3-cosmos-rascal/** - plugin that defines the RASCAL target and telemetry packet
* **satellite/** - rust programs that stand in for the satellite (one cargo workspace)
  * `eps_gen` - sends simulated EPS (power system) telemetry
  * `adcs_gen` - sends simulated ADCS (attitude) telemetry
  * `uplink_listener` - receives encrypted commands from OpenC3 and sends an encrypted ACK back


## prerequisites
install the following
* **docker desktop** 
* **Rust**

## for installation
(for security local .env are ignored)
To install openc3

1. navigate to:
``` cd cosmos-project ```

2. create local env from the template I have included
``` cp .env.example .env ```

3. launch openc3
* **linux/mac** 
``` ./openc3.sh run ```
* **windows**
``` openc3.bat run ```

`compose.override.yaml` is already in the repo. It opens UDP port 6900 so our rust programs can reach OpenC3. If you change it, restart OpenC3 (`./openc3.sh stop`, then `run`)

4. once ran, visit:
**`http://localhost:2900`**

## build and install RASCAL plugin
OpenC3 does not, by default, know our telemetry packet is there, but it will once the plugin is installed.

1. build it
from (cosmos-project/plugins/openc3-cosmos-rascal):

* **linux/mac**
```bash
cd plugins/openc3-cosmos-rascal
../../openc3.sh cli rake build VERSION=1.0.0
```
* **windows**
```bat
cd plugins\openc3-cosmos-rascal
..\..\openc3.bat cli rake build VERSION=1.0.0
```
This creates `openc3-cosmos-rascal-1.0.0.gem` in that folder.
 
NOTE: Upon new VERSION1.x.x, click "UPDATE" in OpenC3 under Admin>Installed plugins.

2. in OpenC3 go to Admin > Plugins > Install from file choose `.gem`, check that `rascal_tlm_port`is **6900** and `rascal_cmd_port` is **6901** and `rascal_cmd_host` is **host.docker.internal** then click **Install**

NOTE: host.docker.internal is how OpenC3, inside docker, reaches the rust programs on your computer

3. RASCAL should now be viewable in the Installed Plugins list

NOTE: after a pull request the plugin may change, rebuild with a higher VERSION (1.0.1, 1.0.2, ...) and in Admin > Plugins click `...` > **Upgrade** on RASCAL. Don't use Install from file again, it fails with "already exists".

4. run telemetry from command line

**EPS generator** (see "view the EPS graphs" below)
```bash
cd satellite
cargo run --release -p eps_gen
```

## view the telemetry
1. open **Packet Viewer** from the OpenC3 tool menu
2. select target **RASCAL**, packet **EPS**
3. `COUNTER` should match that in your command line

to check it is arriving, open **CmdTlmServer**:
* **Interfaces** tab: `RASCAL_INT` should be CONNECTED and its packet count should go up
* **Packets** tab: `RASCAL EPS` received count should go up


## view the EPS graphs
The RASCAL plugin includes an **EPS_GRAPHS** screen (`targets/RASCAL/screens/eps_graphs.txt`) with live graphs of battery voltage, battery + solar current and battery temperature. EPS values also have limits in `tlm.txt`, so they turn yellow/red when out of range.

If you installed the plugin before this screen was added, rebuild and upgrade it once:

1. pull the latest main: `git pull origin main`
2. rebuild with a higher version, from `cosmos-project/plugins/openc3-cosmos-rascal`:
   * **linux/mac** `../../openc3.sh cli rake build VERSION=1.0.1`
   * **windows** `..\..\openc3.bat cli rake build VERSION=1.0.1`
3. in OpenC3 go to Admin > Plugins, click **Upgrade** on openc3-cosmos-rascal and choose the new `.gem`

To view the graphs:

1. run the EPS generator
```bash
cd genData/eps_gen
cargo run --release
```
2. open **Telemetry Viewer**, select target **RASCAL**, screen **EPS_GRAPHS**, click **Show**

eps_gen sends one 90 minute orbit in 90 seconds (1 packet per second = 60 s of orbit), then stops. Run it again to replay.


## send a command
commands go from OpenC3 to the satellite program, encrypted with AES-256-GCM.
 
1. start the satellite command receiver:
```bash
cd satellite
cargo run --release -p uplink_listener
```
2. in OpenC3 open **Command Sender**, select **RASCAL NOOP**, and send it
3. the listener should print that authentication succeeded, `NOOP received with sequence number N`, and `Encrypted ACK sent`
4. in OpenC3 check the **ACK** packet (Packet Viewer > RASCAL > ACK, or CmdTlmServer > Packets). Its `SEQ_NUM` should match the command
 
You can run `uplink_listener` and a telemetry sender at the same time.

## packet format
big-endian, once per second.
NOTE: must match what is inside of 'mock_lore_forc3/src/main.rs` and `cosmos-project/plugins/openc3-cosmos-rascal-targets/RASCAL/cmd_tlm/tlm.txt`

telemetry (satellite to OpenC3), first byte is the packet ID:
* **HEALTH** (ID 1): COUNTER (u16)
* **EPS** (ID 2): COUNTER (u16), SIM_TIME (u32, s), BATT_VOLTAGE (u16, mV), BATT_CURRENT (i16, mA), BATT_TEMP (i16, 0.01 C), SOLAR_CURRENT (i16, mA)
* **ACK** (ID 3): SEQ_NUM (u8), sent encrypted
 
commands (OpenC3 to satellite):
* **NOOP** (ID 1): SEQ_NUM (u8)
 
encrypted command: `[version=1][direction=1]` (2 byte header, authenticated) + 12 byte nonce + ciphertext + 16 byte tag. plaintext is `[CMD_ID][SEQ_NUM]`

encrypted ACK: `[0xA5][version=1][direction=2]` (3 byte header) + 12 byte nonce + ciphertext + 16 byte tag. plaintext is `[3][SEQ_NUM]`



## ports
port | use 
2900 | OpenC3 web UI
6900/udp | telemetry into OpenC3
6901/udp | commands out of OpenC3 
7799-7814/udp | reserved by OpenC3