//! Arm TrustZone-specific material for Sinaloa
//!
//! ## Authors
//!
//! The Veracruz Development Team.
//!
//! ## Licensing and copyright notice
//!
//! See the `LICENSE.markdown` file in the Veracruz root directory for
//! information on licensing and copyright.

#[cfg(feature = "tz")]
pub mod sinaloa_tz {

    use crate::sinaloa::*;
    use hex;
    use lazy_static::lazy_static;
    use log::debug;
    use optee_teec::{
        Context, Operation, ParamNone, ParamTmpRef, ParamType, ParamValue, Session, Uuid,
    };
    use std::convert::TryInto;
    use std::sync::Mutex;
    use veracruz_utils::{JaliscoOpcode, MCOpcode, JALISCO_UUID, MC_UUID};

    lazy_static! {
        static ref CONTEXT: Mutex<Option<Context>> = Mutex::new(Some(Context::new().unwrap()));
        static ref JALISCO_INITIALIZED: Mutex<bool> = Mutex::new(false);
    }

    pub struct SinaloaTZ {
        mexico_city_uuid: String,
    }

    impl Sinaloa for SinaloaTZ {
        fn new(policy_json: &str) -> Result<Self, SinaloaError> {
            // Set up, initialize Jalisco
            //let jalisco_uuid = Uuid::parse_str("8aaaf200-2450-11e4-abe2-0002a5d5c51b").unwrap();
            let policy: veracruz_utils::VeracruzPolicy =
                veracruz_utils::VeracruzPolicy::from_json(policy_json)?;

            let jalisco_uuid = Uuid::parse_str(&JALISCO_UUID.to_string())?;
            {
                let mut ji_guard = JALISCO_INITIALIZED.lock()?;
                if !*ji_guard {
                    debug!("Jalisco is uninitialized.");
                    SinaloaTZ::native_attestation(
                        &policy.tabasco_url(),
                        jalisco_uuid,
                        &policy.mexico_city_hash(),
                    )?;
                    *ji_guard = true;
                }
            }

            let mc_uuid = Uuid::parse_str(&MC_UUID.to_string())?;

            let p0 = ParamTmpRef::new_input(&policy_json.as_bytes());

            let mut operation = Operation::new(0, p0, ParamNone, ParamNone, ParamNone);

            {
                let mut context_opt = CONTEXT.lock()?;
                let context = context_opt
                    .as_mut()
                    .ok_or(SinaloaError::UninitializedEnclaveError)?;
                let mut mc_session = context.open_session(mc_uuid)?;
                mc_session.invoke_command(MCOpcode::Initialize as u32, &mut operation)?;
            }

            Ok(Self {
                mexico_city_uuid: MC_UUID.to_string(),
            })
        }

        fn plaintext_data(&self, data: Vec<u8>) -> Result<Option<Vec<u8>>, SinaloaError> {
            let parsed = colima::parse_mexico_city_request(&data)?;

            if parsed.has_request_proxy_psa_attestation_token() {
                let rpat = parsed.get_request_proxy_psa_attestation_token();
                let challenge = colima::parse_request_proxy_psa_attestation_token(rpat);
                let (psa_attestation_token, pubkey, device_id) =
                    self.proxy_psa_attestation_get_token(challenge)?;
                let serialized_pat = colima::serialize_proxy_psa_attestation_token(
                    &psa_attestation_token,
                    &pubkey,
                    device_id,
                )?;
                Ok(Some(serialized_pat))
            } else {
                Err(SinaloaError::MissingFieldError(
                    "plaintext_data proxy_psa_attestation_toke",
                ))
            }
        }

        // Note: this function will go away
        fn get_enclave_cert(&self) -> Result<Vec<u8>, SinaloaError> {
            let mut context_opt = CONTEXT.lock()?;
            let context = context_opt
                .as_mut()
                .ok_or(SinaloaError::UninitializedEnclaveError)?;
            let mc_uuid = Uuid::parse_str(&self.mexico_city_uuid)?;
            let mut session = context.open_session(mc_uuid)?;

            // get the certificate size
            let certificate_len = {
                let p0 = ParamValue::new(0, 0, ParamType::ValueOutput);
                let mut operation = Operation::new(0, p0, ParamNone, ParamNone, ParamNone);

                session.invoke_command(MCOpcode::GetEnclaveCertSize as u32, &mut operation)?;
                operation.parameters().0.a()
            };

            let certificate = {
                let mut cert_vec = vec![0; certificate_len as usize];
                let p0 = ParamTmpRef::new_output(&mut cert_vec);
                let mut operation = Operation::new(0, p0, ParamNone, ParamNone, ParamNone);
                session.invoke_command(MCOpcode::GetEnclaveCert as u32, &mut operation)?;
                cert_vec
            };
            Ok(certificate)
        }

        // Note: This function will go away
        fn get_enclave_name(&self) -> Result<String, SinaloaError> {
            let mut context_opt = CONTEXT.lock()?;
            let context = context_opt
                .as_mut()
                .ok_or(SinaloaError::UninitializedEnclaveError)?;
            let mc_uuid = Uuid::parse_str(&self.mexico_city_uuid)?;
            let mut session = context.open_session(mc_uuid)?;

            // get the enclave name size
            let name_len = {
                let p0 = ParamValue::new(0, 0, ParamType::ValueOutput);
                let mut operation = Operation::new(0, p0, ParamNone, ParamNone, ParamNone);

                session.invoke_command(MCOpcode::GetEnclaveNameSize as u32, &mut operation)?;
                operation.parameters().0.a()
            };
            let name: String = {
                let mut name_vec = vec![0; name_len as usize];
                //let mut name_vec = Vec::with_capacity(name_len as usize);
                let p0 = ParamTmpRef::new_output(&mut name_vec);
                let mut operation = Operation::new(0, p0, ParamNone, ParamNone, ParamNone);
                session.invoke_command(MCOpcode::GetEnclaveName as u32, &mut operation)?;
                String::from_utf8(name_vec)?
            };
            Ok(name)
        }

        fn proxy_psa_attestation_get_token(
            &self,
            challenge: Vec<u8>,
        ) -> Result<(Vec<u8>, Vec<u8>, i32), SinaloaError> {
            let mut token: Vec<u8> = Vec::with_capacity(2 * 8192); // TODO: Don't do
            let mut pubkey = Vec::with_capacity(256); // TODO: Don't do this

            let mut context_opt = CONTEXT.lock()?;
            let context = context_opt
                .as_mut()
                .ok_or(SinaloaError::UninitializedEnclaveError)?;
            let mc_uuid = Uuid::parse_str(&self.mexico_city_uuid)?;
            let mut session = context.open_session(mc_uuid)?;

            // Get the token, public key and device_id
            // p0 - challenge input
            // p1 - device_id output
            // p2 - token output
            // p3 - pubkey output
            let p0 = ParamTmpRef::new_input(&challenge);

            let p1 = ParamValue::new(0, 0, ParamType::ValueOutput);

            //let p1 = ParamValue::new(token.capacity() as u32, 0 as u32, ParamType::ValueInout); // a = token_len, b=device_id
            let p2 = ParamTmpRef::new_output(&mut token);
            let p3 = ParamTmpRef::new_output(&mut pubkey);

            let mut operation = Operation::new(0, p0, p1, p2, p3);
            session.invoke_command(MCOpcode::GetPSAAttestationToken as u32, &mut operation)?;

            let (_, p1_output, p2_output, p3_output) = operation.parameters();
            let device_id: i32 = p1_output.a().try_into()?;
            unsafe {
                let token_len = p2_output.updated_size();
                token.set_len(token_len);
            }
            unsafe {
                let pubkey_len = p3_output.updated_size();
                pubkey.set_len(pubkey_len);
            }
            Ok((token, pubkey, device_id))
        }

        fn new_tls_session(&self) -> Result<u32, SinaloaError> {
            let mut context_opt = CONTEXT.lock()?;
            let context = context_opt
                .as_mut()
                .ok_or(SinaloaError::UninitializedEnclaveError)?;

            let mc_uuid = Uuid::parse_str(&self.mexico_city_uuid)?;
            let mut session = context.open_session(mc_uuid)?;

            let p0 = ParamValue::new(0, 0, ParamType::ValueOutput);
            let mut operation = Operation::new(0, p0, ParamNone, ParamNone, ParamNone);

            session.invoke_command(MCOpcode::NewTLSSession as u32, &mut operation)?;

            let session_id = operation.parameters().0.a();

            Ok(session_id)
        }

        fn close_tls_session(&self, session_id: u32) -> Result<(), SinaloaError> {
            let mut context_opt = CONTEXT.lock()?;
            let context = context_opt
                .as_mut()
                .ok_or(SinaloaError::UninitializedEnclaveError)?;
            let mc_uuid = Uuid::parse_str(&self.mexico_city_uuid)?;
            let mut session = context.open_session(mc_uuid)?;
            let p0 = ParamValue::new(session_id, 0, ParamType::ValueInput);
            let mut operation = Operation::new(0, p0, ParamNone, ParamNone, ParamNone);
            session.invoke_command(MCOpcode::CloseTLSSession as u32, &mut operation)?;
            Ok(())
        }

        fn tls_data(
            &self,
            session_id: u32,
            input: Vec<u8>,
        ) -> Result<(bool, Option<Vec<Vec<u8>>>), SinaloaError> {
            let mut context_opt = CONTEXT.lock()?;
            let context = context_opt
                .as_mut()
                .ok_or(SinaloaError::UninitializedEnclaveError)?;
            let mc_uuid = Uuid::parse_str(&self.mexico_city_uuid)?;
            let mut session = context.open_session(mc_uuid)?;

            {
                let p0 = ParamValue::new(session_id, 0, ParamType::ValueInput);
                let p1 = ParamTmpRef::new_input(&input);
                let mut operation = Operation::new(0, p0, p1, ParamNone, ParamNone);
                session.invoke_command(MCOpcode::SendTLSData as u32, &mut operation)?;
            }

            let mut active_flag = true;
            let mut ret_array = Vec::new();
            while self.tls_data_needed(session_id, &mut session)? {
                let output_size: usize = 100000; // set to ridiculous long length. TODO: Fix this
                let mut output = vec![0; output_size];

                let p0 = ParamValue::new(session_id, 0, ParamType::ValueInout);
                let p1 = ParamTmpRef::new_output(&mut output);
                let p2 = ParamValue::new(0, 0, ParamType::ValueInout);
                let active = ParamValue::new(1, 0, ParamType::ValueInout);
                let mut operation = Operation::new(0, p0, p1, p2, active);

                session.invoke_command(MCOpcode::GetTLSData as u32, &mut operation)?;
                let output_len = operation.parameters().2.a() as usize;
                active_flag = operation.parameters().3.a() != 0;
                ret_array.push(output[0..output_len].to_vec());
            }

            Ok((
                active_flag,
                if ret_array.len() > 0 {
                    Some(ret_array)
                } else {
                    None
                },
            ))
        }

        fn close(&mut self) -> Result<bool, SinaloaError> {
            let mut context_guard = CONTEXT.lock()?;
            let mc_uuid = Uuid::parse_str(&self.mexico_city_uuid)?;
            match &mut *context_guard {
                None => {
                    return Err(SinaloaError::UninitializedEnclaveError);
                }
                Some(context) => {
                    let mut session = context.open_session(mc_uuid)?;
                    let mut null_operation =
                        Operation::new(0, ParamNone, ParamNone, ParamNone, ParamNone);
                    session.invoke_command(MCOpcode::ResetEnclave as u32, &mut null_operation)?;
                }
            }

            Ok(true)
        }
    }

    impl Drop for SinaloaTZ {
        fn drop(&mut self) {
            match self.close() {
                // We can only panic here since drop function cannot return.
                Err(err) => panic!("SinaloaTZ::drop failed in call to self.close:{:?}", err),
                _ => (),
            }
        }
    }

    impl SinaloaTZ {
        fn tls_data_needed(
            &self,
            session_id: u32,
            session: &mut Session,
        ) -> Result<bool, SinaloaError> {
            let p0 = ParamValue::new(session_id, 0, ParamType::ValueInout);
            let mut operation = Operation::new(0, p0, ParamNone, ParamNone, ParamNone);
            session.invoke_command(MCOpcode::GetTLSDataNeeded as u32, &mut operation)?;
            Ok(operation.parameters().0.b() == 1)
        }

        fn native_attestation(
            tabasco_url: &String,
            jalisco_uuid: Uuid,
            mexico_city_hash: &String,
        ) -> Result<(), SinaloaError> {
            let mut context_opt = CONTEXT.lock()?;
            let context = context_opt
                .as_mut()
                .ok_or(SinaloaError::UninitializedEnclaveError)?;
            let mut jalisco_session = context.open_session(jalisco_uuid)?;

            let firmware_version = SinaloaTZ::fetch_firmware_version(&mut jalisco_session)?;

            {
                let mexico_city_hash_vec = hex::decode(mexico_city_hash.as_str())?;
                let p0 = ParamTmpRef::new_input(&mexico_city_hash_vec);
                let mut operation = Operation::new(0, p0, ParamNone, ParamNone, ParamNone);
                jalisco_session
                    .invoke_command(JaliscoOpcode::SetMexicoCityHashHack as u32, &mut operation)?;
            }
            let (challenge, device_id) =
                SinaloaTZ::send_start(tabasco_url, "psa", &firmware_version)?;

            let p0 = ParamValue::new(device_id.try_into()?, 0, ParamType::ValueInout);
            let p1 = ParamTmpRef::new_input(&challenge);
            let mut token: Vec<u8> = vec![0; 1024]; //Vec::with_capacity(1024); // TODO: Don't do this
            let p2 = ParamTmpRef::new_output(&mut token);
            let mut public_key: Vec<u8> = Vec::with_capacity(128); // TODO: Don't do this
            let p3 = ParamTmpRef::new_output(&mut public_key);
            let mut na_operation = Operation::new(0, p0, p1, p2, p3);
            jalisco_session
                .invoke_command(JaliscoOpcode::NativeAttestation as u32, &mut na_operation)?;
            let token_size = na_operation.parameters().0.b();
            let public_key_size = na_operation.parameters().0.a();
            let token_vec: Vec<u8> = token[0..token_size as usize].to_vec();
            unsafe { public_key.set_len(public_key_size as usize) };

            SinaloaTZ::post_native_psa_attestation_token(tabasco_url, &token_vec, device_id)?;
            debug!("sinaloa_tz::native_attestation returning Ok");
            return Ok(());
        }

        fn post_native_psa_attestation_token(
            tabasco_url: &String,
            token: &Vec<u8>,
            device_id: i32,
        ) -> Result<(), SinaloaError> {
            debug!("sinaloa_tz::post_psa_attestation_token started");
            let serialized_tabasco_request =
                colima::serialize_native_psa_attestation_token(token, device_id)?;
            let encoded_str = base64::encode(&serialized_tabasco_request);
            let url = format!("{:}/PSA/AttestationToken", tabasco_url);
            let response = crate::post_buffer(&url, &encoded_str)?;

            debug!(
                "sinaloa_tz::post_psa_attestation_token received buffer:{:?}",
                response
            );
            return Ok(());
        }

        fn fetch_firmware_version(jal_session: &mut Session) -> Result<String, SinaloaError> {
            let firmware_version_len = {
                let p0 = ParamValue::new(0, 0, ParamType::ValueOutput);
                let mut gfvl_op = Operation::new(0, p0, ParamNone, ParamNone, ParamNone);
                jal_session
                    .invoke_command(JaliscoOpcode::GetFirmwareVersionLen as u32, &mut gfvl_op)?;
                gfvl_op.parameters().0.a()
            };
            let firmware_version: String = {
                let mut fwv_vec = vec![0; firmware_version_len as usize];
                let p0 = ParamTmpRef::new_output(&mut fwv_vec);
                let mut gfv_op = Operation::new(0, p0, ParamNone, ParamNone, ParamNone);
                jal_session
                    .invoke_command(JaliscoOpcode::GetFirmwareVersion as u32, &mut gfv_op)?;
                String::from_utf8(fwv_vec)?
            };
            return Ok(firmware_version);
        }

        fn send_start(
            url_base: &str,
            protocol: &str,
            firmware_version: &str,
        ) -> Result<(Vec<u8>, i32), SinaloaError> {
            let tabasco_response = crate::send_tabasco_start(url_base, protocol, firmware_version)?;
            if tabasco_response.has_psa_attestation_init() {
                let (challenge, device_id) = colima::parse_psa_attestation_init(
                    tabasco_response.get_psa_attestation_init(),
                )?;
                Ok((challenge, device_id))
            } else {
                Err(SinaloaError::MissingFieldError(
                    "tabasco_response psa_attestation_init",
                ))
            }
        }
    }
}
