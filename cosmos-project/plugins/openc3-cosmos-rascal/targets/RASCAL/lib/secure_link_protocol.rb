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

      @ack_key = [
        '102132435465768798a9bacbdcedfe0f' \
        '102132435465768798a9bacbdcedfe0f'
       ].pack('H*')




    end

    def read_data(data, extra = nil)
        return super(data, extra) if data.empty?

        first_byte = data.getbyte(0)

        # Existing HEALTH and EPS and ADCS telemetry are plaintext.
        if first_byte == 1 || first_byte == 2 || first_byte == 4
            return data, extra
        end

        # Encrypted ACKs must start with our marker.
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
            "[RASCAL SECURITY] Encrypted ACK too short"
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
            "[RASCAL SECURITY] Invalid ACK version or direction"
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

            decipher.key = @ack_key
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

        # ACK must contain [PKT_ID=3][SEQ_NUM].
        if plaintext.bytesize != 2 || plaintext.getbyte(0) != 3
            OpenC3::Logger.warn(
            "[RASCAL SECURITY] Invalid decrypted ACK format"
            )
            return nil, extra
        end

        OpenC3::Logger.info(
            "[RASCAL SECURITY] ACK authenticated: #{plaintext.unpack1('H*')}"
        )

        return plaintext, extra
    end


    def write_data(data, extra = nil)

      if data.empty?
        return super(data, extra)
      end

      # Create a fresh 12-byte nonce for this encryption.
      nonce = SecureRandom.random_bytes(12)

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
  end
end
