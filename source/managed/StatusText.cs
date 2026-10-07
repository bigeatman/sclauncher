using System;

namespace ScMultiTest
{
    internal static class StatusText
    {
        internal static string Explain(string message)
        {
            if (!String.IsNullOrEmpty(message))
            {
                if (message.StartsWith("Waiting for in-game UI callbacks", StringComparison.Ordinal))
                    return "메뉴·로비 연결 대기 · 테스트 게임에 들어가면 연결을 이어갑니다.";
                if (message.StartsWith("Game UI callbacks stayed empty", StringComparison.Ordinal))
                    return "게임 안에서도 연결 위치가 준비되지 않았습니다. 초기화 오류 기록을 확인하세요.";
            }
            switch (message)
            {
                case "Unit control ready; larva and rally fix 20261005; live validation pending":
                case "Unit control ready; production fix 20261004; live validation pending": return "유닛 제어 준비됨 · 게임에서 선택 후 백틱(`)";
                case "Connect and enter a game": case "Waiting for a live game":
                case "Waiting for game state": return "게임에 들어간 뒤 같은 종류의 본인 유닛 1개 또는 여러 개를 선택하세요.";
                case "Analyzing loaded game image": return "게임 연결 위치를 확인하고 있습니다.";
                case "Select exactly one owned unit first": return "같은 종류의 본인 유닛 1개 또는 여러 개를 선택한 뒤 백틱(`)을 누르세요.";
                case "Click another owned unit once, then press backtick": return "같은 종류의 본인 유닛을 다시 선택한 뒤 백틱(`)을 누르세요.";
                case "Select owned same-type units first": return "선택된 유닛이 없습니다. 같은 종류의 본인 유닛을 선택한 뒤 백틱(`)을 누르세요.";
                case "Selected units have different types": return "서로 다른 종류의 유닛이 선택되어 있습니다. 같은 종류만 선택한 뒤 백틱(`)을 누르세요.";
                case "Selected units are not all owned by local player": return "다른 플레이어의 유닛이 포함되어 있습니다. 본인 유닛만 선택한 뒤 백틱(`)을 누르세요.";
                case "Selection read failed; select owned same-type units again": return "현재 선택을 읽지 못했습니다. 같은 종류의 본인 유닛을 다시 선택한 뒤 백틱(`)을 누르세요.";
                case "Selected unit IDs are invalid or duplicated": return "선택된 유닛의 식별 정보가 올바르지 않거나 중복되어 있습니다. 유닛을 다시 선택하세요.";
                case "Selection ID encoding unavailable; click selected units again": return "선택된 유닛의 식별 방식을 확인하지 못했습니다. 유닛을 다시 선택한 뒤 백틱(`)을 누르세요.";
                case "Selection array address is unavailable": return "선택 배열 주소를 읽지 못했습니다. 선택 진단 기록을 확인하세요.";
                case "Selection slot memory read failed": return "선택 슬롯을 읽지 못했습니다. 선택 진단 기록을 확인하세요.";
                case "Selected unit pointers are duplicated": return "선택 정보에 같은 유닛이 중복되어 있습니다. 선택 진단 기록을 확인하세요.";
                case "Selected unit pointer is outside unit array": return "선택된 유닛 주소가 유닛 배열 범위를 벗어났습니다. 선택 진단 기록을 확인하세요.";
                case "Selected unit memory read failed": return "선택된 유닛의 메모리를 읽지 못했습니다. 선택 진단 기록을 확인하세요.";
                case "Selected unit sprite is unavailable": return "선택된 유닛의 화면 정보가 없습니다. 선택 진단 기록을 확인하세요.";
                case "Selected unit is dead or has no live order": return "선택된 유닛이 죽었거나 활성 상태가 아닙니다.";
                case "Selected unit is not completed": return "선택된 유닛이 완성 상태로 확인되지 않습니다. 선택 진단 기록을 확인하세요.";
                case "Selected unit is inside transport": return "선택된 유닛이 수송 중입니다. 밖에 있는 유닛을 선택하세요.";
                case "Selected unit type or owner is invalid": return "선택된 유닛의 종류나 소유자 정보가 올바르지 않습니다. 선택 진단 기록을 확인하세요.";
                case "Unit array layout unavailable": return "유닛 목록의 구조를 확인하지 못했습니다. 게임 연결 오류 기록을 확인하세요.";
                case "Matching unit list unavailable": return "같은 종류의 본인 유닛 목록을 읽지 못했습니다. 다시 선택한 뒤 백틱(`)을 누르세요.";
                case "Selected units are missing from matching unit list": return "선택된 유닛이 같은 종류의 본인 유닛 목록과 일치하지 않습니다. 유닛을 다시 선택하세요.";
                case "Select a newly created same-type unit and try again": return "유닛 목록을 확인할 수 없습니다. 새로 생성된 유닛으로 다시 시도하세요.";
                case "Select one completed owned building first": return "완성된 본인 건물 1개를 선택한 뒤 백틱(`)을 누르세요.";
                case "Building control requires one original building; group cleared": return "건물은 기준 건물 1개를 선택해야 합니다. 전체 제어를 해제했습니다.";
                case "Unit command metadata unavailable; group cleared": return "생산·랠리 명령에 필요한 종류 정보를 확인하지 못해 전체 제어를 해제했습니다.";
                case "Unsupported production or rally for selected type; group cleared": return "이 종류는 전체 생산·랠리 대상이 아닙니다. 전체 제어를 해제했습니다.";
                case "No controllable matching units": return "제어 가능한 같은 종류의 유닛이 없습니다.";
                case "Matching owned units selected": return "같은 종류의 전체 제어가 켜졌습니다.";
                case "Control queued for matching owned units": return "같은 종류의 유닛에 명령을 적용하고 있습니다.";
                case "Batch appended to game outgoing buffer": return "명령 묶음을 게임 버퍼에 추가했습니다.";
                case "Manual selection; group cleared": return "새 선택으로 전체 제어를 해제했습니다.";
                case "Backtick toggled off": return "백틱(`)으로 전체 제어를 해제했습니다.";
                case "Escape pressed; group cleared": return "Esc로 전체 제어를 해제했습니다.";
                case "Stopped by test app": return "프로그램에서 전체 제어를 해제했습니다.";
                case "Game lost focus; group cleared": return "다른 창으로 전환하여 전체 제어를 해제했습니다.";
                case "Test app inactive; group cleared": return "프로그램 연결이 끊겨 전체 제어를 해제했습니다.";
                case "Selection changed during control; group cleared":
                case "Selection changed without a control; group cleared":
                case "Selection changed; group cleared": return "선택이 변경되어 전체 제어를 해제했습니다.";
                case "Game changed; selection cleared": case "Local player changed; group cleared":
                    return "게임이나 플레이어가 변경되어 전체 제어를 해제했습니다.";
                case "Original morph identity changed; group cleared":
                case "Reference unit no longer controllable": case "Reference unit removed; group cleared":
                case "Selected unit is unavailable": return "기준 유닛을 제어할 수 없어 전체 제어를 해제했습니다.";
                case "Unsupported control; group cleared": return "이 명령은 전체 제어 대상에 포함되지 않습니다.";
                case "Queued command expired; group cleared": return "명령 대기 시간이 초과되어 전체 제어를 해제했습니다.";
                case "Too many queued commands; group cleared": return "대기 명령이 너무 많아 전체 제어를 해제했습니다.";
                case "Matching units unavailable": return "같은 종류의 유닛이 없어 전체 제어를 해제했습니다.";
                case "Callback failed; command cloning disabled": return "게임 연결 오류로 전체 제어를 중지했습니다. 게임을 재시작하세요.";
                case "Outgoing sender rejected batch; no more copies": return "게임이 명령 묶음을 받지 않아 전체 제어를 중지했습니다.";
                case "Observed control queued for other matching owned units": return "추가 유닛에 보낼 명령을 대기열에 넣었습니다.";
                case "Original larvae already received production; group cleared": return "선택한 라바에 생산 명령이 적용되어 전체 제어를 해제했습니다.";
                case "Original selection already received control": return "추가로 명령을 보낼 유닛이 없습니다.";
                case "Callback table reset; observation gap cleared group": return "게임의 연결 상태가 바뀌어 해제했습니다. 백틱을 다시 누르세요.";
                case "Callback table changed; copying disabled": return "게임 연결 위치가 달라져 명령 복제를 중지했습니다.";
                case "Callback snapshot guard failed; group cleared": return "명령 상태를 확인할 수 없어 전체 제어를 해제했습니다.";
                case "Callback context changed; group cleared": return "명령 처리 중 게임 상태가 바뀌어 해제했습니다.";
                case "Ambiguous or unsupported callback output; group cleared": return "지원되지 않거나 불명확한 입력으로 전체 제어를 해제했습니다.";
                case "Unsupported callback control; group cleared": return "지원하지 않는 명령으로 전체 제어를 해제했습니다.";
                case "Client selection changed; group cleared":
                case "Client selection changed before batch; group cleared": return "기준 유닛 선택이 바뀌어 전체 제어를 해제했습니다.";
                default: return message;
            }
        }
    }
}