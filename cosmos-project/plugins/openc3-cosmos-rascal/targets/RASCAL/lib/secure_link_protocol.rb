#defined our own openC3 protocol
#protocol can transform it into the form OpenC3 expects.
#OpenC3 protocols sit between the interface and the packet-processing pipeline.
#write_data can transform outgoing command bytes
#read_data can transform incoming bytes
#all before OpenC3 decodes them 
#he super call initializes the base protocol's state and preserves OpenC3's normal handling of empty input

require 'openc3/interfaces/protocols/protocol' #import protocol frameword that c3 provides
require 'openssl'
require 'securerandom'


module OpenC3
  class SecureLinkProtocol < Protocol  #inhertits from protocol


    VERSION = 1
    COMMAND_DIRECTION = 1  # Ground station to Sattellite
    ACK_DIRECTION = 2 #Satellite to Ground Station
    ACK_MARKER = 0xA5




    def initialize(allow_empty_data = nil)

        super(allow_empty_data)  # invokes the parent class's constructor
        # Temporary 32-byte development key.
        # Rust must use the same key to decrypt commands.
        @command_key = [
            '00112233445566778899aabbccddeeff' \
            '00112233445566778899aabbccddeeff'
        ].pack('H*')

        @downlink_key = [
            '3CA7BD16E145D141CCCC28BC2C012B18047D7FC1C02FE43EEB17E6727756A6D8'
        ].pack('H*')

        # Persistent command nonce counter configuration.
        @command_nonce_prefix = [0, 0, 0, 4].pack('C4')

        @command_counter_path = ENV.fetch(
        'RASCAL_COMMAND_COUNTER_PATH',
        '/plugins/rascal_state/command_counter.bin'
        )

    end

    def read_data(data, extra = nil)
        return super(data, extra) if data.empty?

        first_byte = data.getbyte(0)

        # Existing HEALTH and packet ID 5 are plaintext.
        if first_byte == 1 || first_byte == 5
            return data, extra
        end

        # Encrypted downlink must start with our marker.
        if first_byte != ACK_MARKER
            OpenC3::Logger.warn(
            "[RASCAL SECURITY] Unknown incoming packet type"
            )
            return nil, extra
        end

        # ACK packet layout:
        # [marker:1][version:1][direction:1]
        # [nonce:12][ciphertext][tag:16]

        header_len = 3
        nonce_len = 12
        tag_len = 16

        min_len = header_len + nonce_len + tag_len

        if data.bytesize < min_len
            OpenC3::Logger.warn(
            "[RASCAL SECURITY] Encrypted downlink packet too short"
            )
            return nil, extra
        end

        #Ruby's byteslice(start, length) takes two arguments:
        # start: The byte index where extraction begins.
        #length: How many bytes to extract.

        header = data.byteslice(0, header_len) 
        version = header.getbyte(1)
        direction = header.getbyte(2)

        if version != VERSION || direction != ACK_DIRECTION
            OpenC3::Logger.warn(
            "[RASCAL SECURITY] Invalid downlink version or direction"
            )
            return nil, extra
        end

        nonce = data.byteslice(header_len, nonce_len)  

        encrypted_payload = data.byteslice(
            header_len + nonce_len,
            data.bytesize - header_len - nonce_len
        )

        # Ruby OpenSSL expects ciphertext and tag separately.
        ciphertext = encrypted_payload.byteslice(
            0,
            encrypted_payload.bytesize - tag_len
        )

        tag = encrypted_payload.byteslice(-tag_len, tag_len)

        begin  #Think of it as similar to try in Python or C++. We're about to perform an operation that can fail.
            decipher = OpenSSL::Cipher.new('aes-256-gcm')
            decipher.decrypt #set the vipher mode 

            decipher.key = @downlink_key
            decipher.iv = nonce
            decipher.auth_tag = tag
            decipher.auth_data = header

            # Authentication is finalized by decipher.final.
            plaintext = decipher.update(ciphertext) + decipher.final

        rescue OpenSSL::Cipher::CipherError
            OpenC3::Logger.warn(
            "[RASCAL SECURITY] ACK authentication FAILED"
            )
            return nil, extra
        end
        # Determine which telemetry packet was decrypted.
        packet_id = plaintext.getbyte(0)

        case packet_id
            
            when 4
            # ADCS packet: 30 bytes, starting with PKT_ID=4.
            if plaintext.bytesize != 30
                OpenC3::Logger.warn(
                    "[RASCAL SECURITY] Invalid decrypted ADCS length"
                )
                return nil, extra
            end

            OpenC3::Logger.info(
                "[RASCAL SECURITY] ADCS authenticated: #{plaintext.unpack1('H*')}"
            )

            when 3
                # ACK packet: [PKT_ID=3][SEQ_NUM]
                if plaintext.bytesize != 2
                    OpenC3::Logger.warn(
                        "[RASCAL SECURITY] Invalid decrypted ACK length"
                    )
                    return nil, extra
                end

                OpenC3::Logger.info(
                    "[RASCAL SECURITY] ACK authenticated: #{plaintext.unpack1('H*')}"
                )
                

            when 2
                # EPS packet: 15 bytes, starting with PKT_ID=2.
                if plaintext.bytesize != 15
                    OpenC3::Logger.warn(
                        "[RASCAL SECURITY] Invalid decrypted EPS length"
                    )
                    return nil, extra
                end

                OpenC3::Logger.info(
                    "[RASCAL SECURITY] EPS authenticated: #{plaintext.unpack1('H*')}"
                )

            else
                OpenC3::Logger.warn(
                    "[RASCAL SECURITY] Unknown decrypted packet ID: #{packet_id.inspect}"
                )
                return nil, extra
            end

            return plaintext, extra
    end


    def write_data(data, extra = nil)

      if data.empty?
        return super(data, extra)
      end

      # Create a fresh 12-byte nonce for this encryption.
      #nonce = SecureRandom.random_bytes(12)

      # Reserve a unique nonce using the persistent command counter.
      nonce = next_command_nonce

      # Header: protocol version + message direction.
      header = [VERSION, COMMAND_DIRECTION].pack('CC') # Ruby's pack() converts an array of values into a binary string.Since we have two C characters, Ruby packs two values into two bytes.

      cipher = OpenSSL::Cipher.new('aes-256-gcm') #AES-GCM is an authenticated encryption algorithm.
      cipher.encrypt #Set encryption mode
      cipher.key = @command_key
      cipher.iv = nonce
      cipher.auth_data = header #AAD is information that we want to authenticate without encrypting.We want the satellite to read those fields, but we don't want an attacker to change them.The attacker cannot modify the header without invalidating the authentication tag.

      ciphertext = cipher.update(data) + cipher.final #Encrypt data
      tag = cipher.auth_tag(16) #produces an authentication tag. The authentication tag is used by Rust to verify that the encrypted packet hasn't been modified and was generated by someone possessing the correct key.

      # Final UDP packet:
      # [version][direction][nonce][ciphertext][tag]
      packet = header + nonce + ciphertext + tag

      OpenC3::Logger.info(
        "[RASCAL SECURITY] Encrypted packet: #{packet.unpack1('H*')}"
      )

     #OpenC3::Logger.info("[RASCAL SECURITY] write_data called! Outgoing bytes: #{data.unpack1('H*')}")           
     # puts "[RASCAL SECURITY] write_data called!"
     # puts "[RASCAL SECURITY] Outgoing bytes: #{data.unpack1('H*')}" #converts the command bytes into hexadecimal so we can inspect them.

     # Return the encrypted packet to OpenC3 for UDP transmission.
      return packet,extra
    end


    def next_command_nonce
      File.open(@command_counter_path, File::RDWR | File::BINARY) do |file|
        file.flock(File::LOCK_EX)

        begin
          raise "Invalid counter file size" unless file.size == 8
          file.rewind
          counter = file.read(8).unpack1('Q>') #This reads an unsigned 64-bit integer in big-endian format, matching the counter layout used by your Rust nonce manager.
          raise "Command nonce counter exhausted" if counter == 0xFFFFFFFFFFFFFFFF
          next_counter = counter + 1

          file.rewind
          file.write([next_counter].pack('Q>'))
          file.flush
          file.fsync
          @command_nonce_prefix + [counter].pack('Q>')
        ensure
          file.flock(File::LOCK_UN)
        end
      end
    end

  end  # Closes SecureLinkProtocol class
end    # Closes OpenC3 module



