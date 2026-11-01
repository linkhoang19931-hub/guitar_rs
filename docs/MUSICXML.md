# Tập bài MusicXML trong quickRS

Bản này (repo `guitar_rs`) là quickRS thêm phần **nhập bài MusicXML**: mọi file tab `.musicxml` / `.mxl`
(xuất từ Guitar Pro, MuseScore, TuxGuitar, hoặc các bài tập B01, B02…) chơi được như một bài Rocksmith —
tab cuộn, vòng lặp, giảm tốc, chấm điểm từng nốt.

## Cách dùng

1. Tải bản Mac ở mục **Releases** (hoặc **Actions → build-mac → Artifacts**), mở `.dmg`, kéo `quickRS.app`
   vào Applications. Lần đầu mở: System Settings → Privacy & Security → **Open Anyway**.
2. Đặt các file `.musicxml` vào một thư mục (thư mục con cũng được), rồi **kéo thư mục đó vào cửa sổ quickRS**.
   Thư mục được giữ lại trong thư viện; lần sau thêm bài mới vào thư mục là tự hiện.
   - Có thể kéo thẳng một file `.musicxml` vào cửa sổ để mở ngay.
   - Thư mục bài của StringTheory (`~/Library/Application Support/StringTheory/StringTheory/Songs`) kéo vào
     được luôn.
3. Chọn bài, chọn bè (Lead / Rhythm / Bass), bấm chơi.

Lần đầu thấy một file MusicXML, quickRS tạo một file `.feedpak` cùng tên nằm ngay cạnh nó. Sửa file MusicXML
thì lần quét sau `.feedpak` được tạo lại. Có thể xoá `.feedpak` bất cứ lúc nào, nó sẽ được tạo lại.

Dòng lệnh (không bắt buộc):

```sh
/Applications/quickRS.app/Contents/MacOS/quickrs-cli musicxml "B03.musicxml"
```

## Bài được chuyển thế nào

| Trong MusicXML | Trong quickRS |
|---|---|
| Mỗi part có tab (`<string>` + `<fret>`) | Một bè: guitar đầu tiên là **Lead**, thứ hai **Rhythm**, part 4 dây là **Bass** |
| `<staff-tuning>`, capo | Tuning (Drop D, Eb…) và capo |
| Tốc độ (`<metronome>` / `<sound tempo>`) | Tốc độ, kể cả đổi tốc độ giữa bài |
| Dấu nhắc lại, khung kết 1./2. | Trải ra đúng thứ tự chơi |
| Nốt nối (tie) | Một nốt ngân dài |
| Nhiều nốt cùng lúc | Hợp âm; tên lấy từ ký hiệu hợp âm trong bài, không có thì tự đoán (E5, Am, G7…) |
| Hammer-on / pull-off, slide, bend, vibrato, harmonic, nốt câm (x), accent, tap, palm mute | Kỹ thuật tương ứng |
| Dấu đoạn (rehearsal mark: A, B, Verse…) | Các đoạn để nhảy và lặp; không có thì mỗi 4 ô một đoạn |

Âm thanh: bài MusicXML không có bản thu, nên quickRS tạo hai kênh:

- **Guitar (synth)**: tiếng guitar tổng hợp chơi đúng tab.
- **Click**: tiếng gõ nhịp, có **một ô đếm vào** trước khi bài bắt đầu.

Hai kênh tách riêng: trong bộ trộn có thể tắt tiếng guitar để chỉ còn tiếng nhịp và tự chơi.

## Giới hạn

- File phải có tab. Bản nhạc chỉ có khuông nhạc thường (không có số dây/phím) sẽ báo lỗi.
- Chỉ đọc dạng `score-partwise` (dạng phổ biến; Guitar Pro, MuseScore đều xuất dạng này).
- Ngón bấm của hợp âm chưa được ghi (bảng nốt hiện hình hợp âm nhưng không có số ngón).
- Chưa đọc: D.S./D.C./Coda, palm mute ghi bằng chữ "P.M." kéo dài nhiều nốt, bend nhiều đoạn.
