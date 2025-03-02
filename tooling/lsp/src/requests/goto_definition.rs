use std::future::{self, Future};

use crate::resolve_workspace_for_source_path;
use crate::{types::GotoDefinitionResult, LspState};
use async_lsp::{ErrorCode, ResponseError};
use fm::codespan_files::Error;
use lsp_types::{GotoDefinitionParams, GotoDefinitionResponse, Location};
use lsp_types::{Position, Url};
use nargo::insert_all_files_for_workspace_into_file_manager;
use noirc_driver::file_manager_with_stdlib;

pub(crate) fn on_goto_definition_request(
    state: &mut LspState,
    params: GotoDefinitionParams,
) -> impl Future<Output = Result<GotoDefinitionResult, ResponseError>> {
    let result = on_goto_definition_inner(state, params);
    future::ready(result)
}

fn on_goto_definition_inner(
    _state: &mut LspState,
    params: GotoDefinitionParams,
) -> Result<GotoDefinitionResult, ResponseError> {
    let file_path =
        params.text_document_position_params.text_document.uri.to_file_path().map_err(|_| {
            ResponseError::new(ErrorCode::REQUEST_FAILED, "URI is not a valid file path")
        })?;

    let workspace = resolve_workspace_for_source_path(file_path.as_path()).unwrap();
    let package = workspace.members.first().unwrap();

    let package_root_path: String = package.root_dir.as_os_str().to_string_lossy().into();

    let mut workspace_file_manager = file_manager_with_stdlib(&workspace.root_dir);
    insert_all_files_for_workspace_into_file_manager(&workspace, &mut workspace_file_manager);

    let (mut context, crate_id) = nargo::prepare_package(&workspace_file_manager, package);

    let interner;
    if let Some(def_interner) = _state.cached_definitions.get(&package_root_path) {
        interner = def_interner;
    } else {
        // We ignore the warnings and errors produced by compilation while resolving the definition
        let _ = noirc_driver::check_crate(&mut context, crate_id, false, false);
        interner = &context.def_interner;
    }

    let files = context.file_manager.as_file_map();
    let file_id = context.file_manager.name_to_id(file_path.clone()).ok_or(ResponseError::new(
        ErrorCode::REQUEST_FAILED,
        format!("Could not find file in file manager. File path: {:?}", file_path),
    ))?;
    let byte_index =
        position_to_byte_index(files, file_id, &params.text_document_position_params.position)
            .map_err(|err| {
                ResponseError::new(
                    ErrorCode::REQUEST_FAILED,
                    format!("Could not convert position to byte index. Error: {:?}", err),
                )
            })?;

    let search_for_location = noirc_errors::Location {
        file: file_id,
        span: noirc_errors::Span::single_char(byte_index as u32),
    };

    let goto_definition_response =
        interner.get_definition_location_from(search_for_location).and_then(|found_location| {
            let file_id = found_location.file;
            let definition_position = to_lsp_location(files, file_id, found_location.span)?;
            let response: GotoDefinitionResponse =
                GotoDefinitionResponse::from(definition_position).to_owned();
            Some(response)
        });

    Ok(goto_definition_response)
}

fn to_lsp_location<'a, F>(
    files: &'a F,
    file_id: F::FileId,
    definition_span: noirc_errors::Span,
) -> Option<Location>
where
    F: fm::codespan_files::Files<'a> + ?Sized,
{
    let range = crate::byte_span_to_range(files, file_id, definition_span.into())?;
    let file_name = files.name(file_id).ok()?;

    let path = file_name.to_string();
    let uri = Url::from_file_path(path).ok()?;

    Some(Location { uri, range })
}

pub(crate) fn position_to_byte_index<'a, F>(
    files: &'a F,
    file_id: F::FileId,
    position: &Position,
) -> Result<usize, Error>
where
    F: fm::codespan_files::Files<'a> + ?Sized,
{
    let source = files.source(file_id)?;
    let source = source.as_ref();

    let line_span = files.line_range(file_id, position.line as usize)?;

    let line_str = source.get(line_span.clone());

    if let Some(line_str) = line_str {
        let byte_offset = character_to_line_offset(line_str, position.character)?;
        Ok(line_span.start + byte_offset)
    } else {
        Err(Error::InvalidCharBoundary { given: position.line as usize })
    }
}

/// Calculates the byte offset of a given character in a line.
/// LSP Clients (editors, eg. neovim) use a different coordinate (LSP Positions) system than the compiler.
///
/// LSP Positions navigate through line numbers and character numbers, eg. `(line: 1, character: 5)`
/// meanwhile byte indexes are used within the compiler to navigate through the source code.
fn character_to_line_offset(line: &str, character: u32) -> Result<usize, Error> {
    let line_len = line.len();
    let mut character_offset = 0;

    let mut chars = line.chars();
    while let Some(ch) = chars.next() {
        if character_offset == character {
            let chars_off = chars.as_str().len();
            let ch_off = ch.len_utf8();

            return Ok(line_len - chars_off - ch_off);
        }

        character_offset += ch.len_utf16() as u32;
    }

    // Handle positions after the last character on the line
    if character_offset == character {
        Ok(line_len)
    } else {
        Err(Error::ColumnTooLarge { given: character_offset as usize, max: line.len() })
    }
}

#[cfg(test)]
mod goto_definition_tests {

    use async_lsp::ClientSocket;
    use tokio::test;

    use crate::solver::MockBackend;

    use super::*;

    #[test]
    async fn test_on_goto_definition() {
        let client = ClientSocket::new_closed();
        let solver = MockBackend;
        let mut state = LspState::new(&client, solver);

        let root_path = std::env::current_dir()
            .unwrap()
            .join("../../test_programs/execution_success/7_function")
            .canonicalize()
            .expect("Could not resolve root path");
        let noir_text_document = Url::from_file_path(root_path.join("src/main.nr").as_path())
            .expect("Could not convert text document path to URI");
        let root_uri = Some(
            Url::from_file_path(root_path.as_path()).expect("Could not convert root path to URI"),
        );

        #[allow(deprecated)]
        let initialize_params = lsp_types::InitializeParams {
            process_id: Default::default(),
            root_path: None,
            root_uri,
            initialization_options: None,
            capabilities: Default::default(),
            trace: Some(lsp_types::TraceValue::Verbose),
            workspace_folders: None,
            client_info: None,
            locale: None,
        };
        let _initialize_response = crate::requests::on_initialize(&mut state, initialize_params)
            .await
            .expect("Could not initialize LSP server");

        let params = GotoDefinitionParams {
            text_document_position_params: lsp_types::TextDocumentPositionParams {
                text_document: lsp_types::TextDocumentIdentifier { uri: noir_text_document },
                position: Position { line: 95, character: 5 },
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let response = on_goto_definition_request(&mut state, params)
            .await
            .expect("Could execute on_goto_definition_request");

        assert!(&response.is_some());
    }
}

#[cfg(test)]
mod character_to_line_offset_tests {
    use super::*;

    #[test]
    fn test_character_to_line_offset() {
        let line = "Hello, dark!";
        let character = 8;

        let result = character_to_line_offset(line, character).unwrap();
        assert_eq!(result, 8);

        // In the case of a multi-byte character, the offset should be the byte index of the character
        // byte offset for 8 character (黑) is expected to be 10
        let line = "Hello, 黑!";
        let character = 8;

        let result = character_to_line_offset(line, character).unwrap();
        assert_eq!(result, 10);
    }
}
