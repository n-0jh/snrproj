
# RASCAL ESP32 Communication Prototype

## Overview

This directory contains the Rust firmware for the RASCAL project's temporary ESP32 development platform.

The ESP32 serves as a hardware prototype for testing communication between a microcontroller and the OpenC3 COSMOS ground station while the team's final CubeSat microcontroller and LoRa hardware are unavailable.

The current implementation supports:

- Rust firmware running on an Xtensa ESP32
- Wi-Fi connectivity using a local 2.4 GHz network
- Automatic IPv4 configuration through DHCP
- Real uptime telemetry generated onboard the ESP32
- Binary telemetry encoding using the RASCAL packet format
- UDP telemetry transmission to the ground station
- Live telemetry decoding and display in OpenC3 COSMOS
- Independent operation using an external USB power supply

**Current status:** Telemetry downlink implemented and verified. Command uplink and encrypted acknowledgments are future milestones.

## 1. Hardware Requirements

### ESP32

Development hardware:

| Property | Value |
|---|---|
| Microcontroller | ESP32-D0WD-V3 |
| Architecture | Xtensa |
| Revision | v3.1 |
| CPU | Dual-core, up to 240 MHz |
| Flash | 4 MB |
| Connectivity | 2.4 GHz Wi-Fi |
| Development interface | USB serial |
| Tested Windows port | COM3 |

The exact serial port may differ between computers.

### Additional Hardware

- USB cable for programming
- USB wall charger or power bank for independent operation
- Windows computer running the RASCAL ground station
- Router providing 2.4 GHz WPA2-Personal Wi-Fi
- Ground station connected to the same reachable LAN

The ground station may use Ethernet while the ESP32 uses Wi-Fi.

No LoRa module or external sensor is required for the current implementation.

## 2. Software Requirements

Install:

1. Rust and Cargo
2. The ESP32 Xtensa Rust toolchain
3. espflash
4. Docker Desktop
5. OpenC3 COSMOS
6. VS Code (recommended)

The project was developed using:

- ESP32 Rust toolchain
- esp-hal 1.2.2
- esp-radio 1.0.0-beta.1
- esp-rtos 0.4.0
- embassy-net 0.9.1
- espflash 4.6.0

The project's Cargo.lock records its resolved Rust dependencies.

### Verify the Toolchain

Run:

```powershell
rustup toolchain list
espflash --version
```

The ESP32 toolchain should appear in the installed toolchains.

### Identify the ESP32

Connect the ESP32 through USB and identify its serial port.

For example:

```powershell
espflash board-info --port COM3
```

This should display the chip type, revision, flash size, and other hardware information.

## 3. Repository Organization

The RASCAL repository contains several separate components:

```text
snrproj/
├── cosmos-project/
│   ├── compose.yaml
│   ├── compose.override.yaml
│   └── plugins/
│       └── openc3-cosmos-rascal/
│           ├── plugin.txt
│           └── targets/
│               └── RASCAL/
│                   ├── cmd_tlm/
│                   │   └── tlm.txt
│                   └── lib/
│                       └── secure_link_protocol.rb
│
├── satellite/
│
└── esp32-firmware/
    ├── .cargo/
    ├── src/
    │   └── bin/
    │       └── main.rs
    ├── Cargo.toml
    ├── Cargo.lock
    ├── rust-toolchain.toml
    └── README.md
```

The ESP32 firmware is maintained as a separate Cargo project because it targets the Xtensa architecture rather than the host computer's architecture.

## 4. Network Configuration

The ESP32 connects to a Wi-Fi network and transmits telemetry to the ground station over UDP.

### Tested Configuration

| Setting | Value |
|---|---|
| ESP32 IPv4 | 192.168.1.142 |
| Ground station IPv4 | 192.168.1.180 |
| Transport | UDP |
| Ground station telemetry port | 6900 |
| ESP32 local telemetry source port | 6902 |
| Telemetry interval | 1 second |

**These addresses are specific to the original development network.**

Teammates must replace the configured ground station address with their own computer's reachable LAN IPv4 address.

The ESP32's DHCP address may change between networks or sessions.

### Determine the Ground Station IP

On Windows, run:

```powershell
ipconfig
```

Find the IPv4 address of the physical Ethernet or Wi-Fi adapter.

Do not use a Docker, WSL, or Hyper-V virtual adapter address as the ESP32's destination unless the network is intentionally configured to route through that adapter.

### Update the Firmware Destination

In:

```text
esp32-firmware/src/bin/main.rs
```

Locate:

```rust
const GROUND_STATION_IP: [u8; 4] = [192, 168, 1, 180];
const TELEMETRY_PORT: u16 = 6900;
```

Replace the IP address with the ground station's address.

Rebuild and reflash the firmware after changing these constants.

## 5. Wi-Fi Credentials

Wi-Fi credentials are supplied to the Rust compiler through environment variables.

In PowerShell:

```powershell
$env:SSID = "YOUR_WIFI_NAME"
$env:PASSWORD = "YOUR_WIFI_PASSWORD"
```

Then compile:

```powershell
cargo build --release
```

The `env!()` macro embeds these values at compile time.

Important considerations:

- Do not commit Wi-Fi passwords to Git.
- Do not commit compiled firmware containing credentials.
- Environment variables must be set again in a new terminal session.
- Changing credentials requires rebuilding and reflashing.
- Credentials embedded in firmware are not securely protected.
- The original ESP32 supports 2.4 GHz Wi-Fi, not 5 GHz.
- WPA2-Personal is the tested authentication configuration.

Campus networks using enterprise authentication or client isolation may require additional configuration.

## 6. Building the ESP32 Firmware

Navigate to the firmware directory:

```powershell
cd esp32-firmware
```

Set the Wi-Fi environment variables as described above.

Build the firmware:

```powershell
cargo build --release
```

The build should produce:

```text
target/xtensa-esp32-none-elf/release/rascal-esp32-wifi
```

Build output is excluded from version control through `.gitignore`.

### Flash the Firmware

Connect the ESP32 through USB.

Run:

```powershell
cargo run --release
```

The project uses `espflash` to upload the firmware and start the serial monitor.

### Manual Bootloader Procedure

The tested development boards did not consistently enter flashing mode automatically.

If `espflash` cannot connect:

1. Hold BOOT.
2. Press and release EN.
3. Release BOOT after the flashing tool connects.
4. Wait until flashing completes.
5. Press EN if the board remains in download mode.

The board should then boot normally.

This manual procedure is a development-board limitation and does not affect normal standalone operation.

## 7. Expected ESP32 Output

After successful flashing, the serial monitor should show messages similar to:

```text
RASCAL ESP32 initialized!
Wi-Fi initialized!
Waiting for network connection...
Connecting to Wi-Fi...
Wi-Fi association successful!
Wi-Fi connected!
ESP32 IP address: 192.168.1.142/24
UDP telemetry socket initialized!
ESP32 telemetry sent: uptime = 1 seconds
ESP32 telemetry sent: uptime = 2 seconds
```

The actual IP address depends on DHCP.

The ESP32 transmits telemetry approximately once per second.

## 8. Telemetry Packet Format

The firmware sends a seven-byte binary packet.

COSMOS identifies the packet using the first byte, which contains packet ID `5`.

### Packet Layout

| Field | Size | Type | Byte order |
|---|---|---|---|
| PKT_ID | 8 bits | Unsigned | N/A |
| COUNTER | 16 bits | Unsigned | Big-endian |
| UPTIME | 32 bits | Unsigned | Big-endian |

Total packet size: **7 bytes**

### Example

For telemetry packet counter 10 and uptime 10 seconds:

```text
05 00 0A 00 00 00 0A
```

Interpretation:

```text
05             Packet ID = 5
00 0A          Counter = 10
00 00 00 0A    Uptime = 10 seconds
```

Rust's `to_be_bytes()` is used to encode the counter and uptime in big-endian order.

### COSMOS Definition

The following definition is included in the RASCAL target's `cmd_tlm/tlm.txt`:

```text
TELEMETRY RASCAL ESP32 BIG_ENDIAN "ESP32 onboard housekeeping telemetry"
  APPEND_ID_ITEM PKT_ID 8 UINT 5 "Packet ID"
  APPEND_ITEM COUNTER 16 UINT "Telemetry packet counter"
  APPEND_ITEM UPTIME 32 UINT "ESP32 uptime in seconds"
    UNITS seconds s
```

## 9. Testing UDP Communication Without COSMOS

Before testing COSMOS integration, telemetry can be received directly in Windows PowerShell.

Ensure COSMOS is not already binding the test port, or use a different available port for the test.

### PowerShell Receiver

```powershell
$udp = New-Object System.Net.Sockets.UdpClient(6900)
$remote = New-Object System.Net.IPEndPoint([System.Net.IPAddress]::Any, 0)

Write-Host "Listening for ESP32 telemetry..."

while ($true) {
    $data = $udp.Receive([ref]$remote)
    $hex = [System.BitConverter]::ToString($data)

    Write-Host "From $($remote.Address): $hex"
}
```

Expected output:

```text
From 192.168.1.142: 05-00-01-00-00-00-01
From 192.168.1.142: 05-00-02-00-00-00-02
From 192.168.1.142: 05-00-03-00-00-00-03
```

Stop the receiver with Ctrl+C.

Close the socket:

```powershell
$udp.Close()
$udp.Dispose()
```

Do not run two receivers bound to the same UDP address and port simultaneously.

## 10. OpenC3 COSMOS Integration

The ground station uses a custom RASCAL COSMOS plugin.

### Telemetry Interface

In `plugin.txt`, the interface is configured using:

```ruby
INTERFACE <%= rascal_target_name %>_INT udp_interface.rb <%= rascal_cmd_host %> <%= rascal_cmd_port %> <%= rascal_tlm_port %>
  PROTOCOL READ_WRITE secure_link_protocol.rb
  MAP_TARGET <%= rascal_target_name %>
```

Relevant settings:

| Variable | Purpose |
|---|---|
| rascal_cmd_host | Command destination |
| rascal_cmd_port | Command destination port, 6901 |
| rascal_tlm_port | Telemetry listening port, 6900 |

The command destination is independent of the telemetry listening port.

### Security Protocol

The custom `secure_link_protocol.rb` processes incoming and outgoing packets.

The current implementation allows these plaintext telemetry packet IDs:

- 1: HEALTH
- 2: EPS
- 4: ADCS
- 5: ESP32

Encrypted acknowledgment packets use a separate authenticated packet format.

The ESP32 telemetry downlink is currently **unencrypted and unauthenticated**.

## 11. Docker UDP Port Mapping

The COSMOS operator runs in Docker.

To receive telemetry from another device on the LAN, Docker must publish the UDP telemetry port on a reachable host interface.

In:

```text
cosmos-project/compose.override.yaml
```

The tested mapping is:

```yaml
- "0.0.0.0:6900:6900/udp"
```

This publishes UDP port 6900 on all host IPv4 interfaces.

### Apply the Configuration

From `cosmos-project`:

```powershell
docker compose up -d --no-deps openc3-operator
```

Verify:

```powershell
docker compose ps
```

The operator should show:

```text
0.0.0.0:6900->6900/udp
```

### Security Consideration

Binding to `0.0.0.0` exposes the UDP port through all available host IPv4 interfaces.

Use this configuration only on a trusted development network and restrict inbound traffic through Windows Firewall.

A more restrictive deployment may bind only to the ground station's LAN address.

## 12. Building the RASCAL COSMOS Plugin

Changes to `tlm.txt` or `secure_link_protocol.rb` must be included in a rebuilt plugin.

The ESP32 telemetry changes were packaged in:

```text
openc3-cosmos-rascal-1.0.6.gem
```

### Build Procedure

From `cosmos-project`:

```powershell
cd .\plugins\openc3-cosmos-rascal
```

Build using the OpenC3 Docker environment:

```powershell
..\..\openc3.bat cli rake build VERSION=1.0.6
```

The build should report:

```text
Successfully built RubyGem
Successfully validated openc3-cosmos-rascal-1.0.6.gem
```

For later updates, increment the plugin version rather than overwriting an existing release.

### Install or Update the Plugin

1. Open COSMOS.
2. Navigate to Admin.
3. Open Plugins.
4. Install or update the RASCAL plugin.
5. Select the newly built `.gem` file.
6. Preserve the correct plugin variables.
7. Wait for installation to complete.

Verify that the RASCAL interface is connected.

## 13. Verifying Telemetry in COSMOS

Open the COSMOS **Packet Viewer**.

Select:

```text
Target: RASCAL
Packet: ESP32
```

Expected fields:

| Field | Expected behavior |
|---|---|
| PKT_ID | Constant value of 5 |
| COUNTER | Increments each packet |
| UPTIME | Increases approximately once per second |

The firmware has been tested transmitting packets to COSMOS through the Windows ground station and Docker UDP forwarding.

Successful operation is confirmed when the Packet Viewer values update continuously.

## 14. Independent Power Test

After flashing, the ESP32 does not need a USB data connection to the ground station.

The firmware remains stored in flash.

To test standalone operation:

1. Leave COSMOS running on the ground station.
2. Disconnect the ESP32 from the computer.
3. Connect the ESP32 to a USB wall charger or power bank.
4. Wait for it to boot and reconnect to Wi-Fi.
5. Monitor the ESP32 packet in COSMOS Packet Viewer.

Expected behavior:

- The ESP32 boots from flash.
- It reconnects to Wi-Fi.
- It obtains an IPv4 address.
- The telemetry counter restarts near zero.
- Live telemetry resumes in COSMOS.

This test was successfully demonstrated using a USB wall charger.

## 15. Troubleshooting

### `xtensa-esp32-elf-gcc` not found

Ensure the Xtensa linker is available in the terminal's PATH.

On the original development computer, the toolchain was installed under:

```text
C:\Users\benji\.rustup\toolchains\esp\xtensa-esp-elf\bin
```

Other computers may use different paths.

### `SSID` or `PASSWORD` not defined

Set the environment variables in the current PowerShell session before compiling.

### ESP32 cannot be flashed

Use the manual BOOT/EN procedure.

Confirm the correct COM port and close other applications using the serial connection.

### ESP32 connects but telemetry does not arrive

Check:

- Ground station IPv4 address
- ESP32 destination IP constant
- UDP destination port
- Wi-Fi and Ethernet network reachability
- Windows Firewall
- Docker UDP port mapping
- COSMOS interface connection status

### UDP port already in use

Stop the previous UDP receiver.

Inspect the port:

```powershell
Get-NetUDPEndpoint -LocalPort 6900
```

COSMOS and a separate PowerShell receiver should not compete for the same host UDP port.

### COSMOS does not recognize the ESP32 packet

Verify that:

- `tlm.txt` defines packet ID 5.
- `secure_link_protocol.rb` accepts packet ID 5.
- The plugin was rebuilt and updated.
- The ESP32 transmits seven-byte binary packets.

### Firmware appears unchanged

Verify the source file on disk:

```powershell
Get-Content .\src\bin\main.rs
```

Save changes before rebuilding and reflashing.

## 16. Current Limitations

The current prototype has several known limitations:

1. Telemetry is not encrypted or authenticated.
2. Wi-Fi credentials are embedded at compile time.
3. The ground station IP address is hardcoded.
4. There is no ESP32 command uplink yet.
5. There are no encrypted ESP32 acknowledgments.
6. UDP does not guarantee packet delivery.
7. Telemetry currently contains uptime and a packet counter.
8. Communication has only been tested over Wi-Fi, not LoRa.
9. The implementation is a development prototype, not flight-qualified firmware.
10. The Wi-Fi implementation is specific to the ESP32.

These limitations will be addressed as development continues.

## 17. Development Roadmap

| Milestone | Objective | Status |
|---|---|---|
| 1 | Program ESP32 in Rust | Complete |
| 2 | Connect ESP32 to Wi-Fi | Complete |
| 3 | Send ESP32 telemetry to COSMOS | Complete |
| 4 | Receive COSMOS commands on ESP32 | Planned |
| 5 | Authenticate AES-GCM commands | Planned |
| 6 | Return encrypted acknowledgments | Planned |
| 7 | Expand real onboard telemetry | Planned |
| 8 | Separate communication logic from Wi-Fi for LoRa integration | Planned |

## 18. Future Architecture

The long-term goal is to separate the application protocol from the physical communication method.

The firmware should eventually contain independent modules for:

- Telemetry generation
- Command processing
- Packet encoding and decoding
- Cryptographic authentication
- Communication transport

This will allow the team to replace Wi-Fi with LoRa without rewriting the application-level command and telemetry handling.

The ESP32 is a temporary development platform. Successful communication over Wi-Fi does not independently validate LoRa range, timing, packet loss, or radio performance.

## 19. Reproduction Checklist

A teammate reproducing the current demonstration should be able to confirm:

- [ ] ESP32 toolchain is installed.
- [ ] ESP32 is detected by espflash.
- [ ] Wi-Fi credentials are configured.
- [ ] Ground station IPv4 address is configured.
- [ ] Firmware builds successfully.
- [ ] Firmware flashes successfully.
- [ ] ESP32 connects to Wi-Fi.
- [ ] ESP32 obtains a DHCP address.
- [ ] Binary telemetry arrives on UDP port 6900.
- [ ] Docker forwards port 6900 to the COSMOS operator.
- [ ] RASCAL plugin includes ESP32 packet ID 5.
- [ ] COSMOS Packet Viewer displays increasing uptime.
- [ ] ESP32 continues operating when powered independently.

## 20. Project Status

This milestone establishes a working, hardware-based telemetry downlink between an independently powered ESP32 and the RASCAL OpenC3 COSMOS ground station.

The next development objective is implementing the command uplink from COSMOS to the ESP32.
