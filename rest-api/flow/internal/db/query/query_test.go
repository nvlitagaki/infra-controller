// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

package Query

import (
	"testing"

	"github.com/DATA-DOG/go-sqlmock"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
	"github.com/uptrace/bun"
	"github.com/uptrace/bun/dialect/pgdialect"
)

func TestNew(t *testing.T) {
	for _, tc := range []struct {
		name  string
		total int64
		count int64
	}{
		{name: "count matching rows", count: 3},
		{name: "reuse supplied total", total: 3, count: 3},
		{name: "count exceeds int32", count: 1 << 33},
	} {
		t.Run(tc.name, func(t *testing.T) {
			sqlDB, mock, err := sqlmock.New(sqlmock.QueryMatcherOption(sqlmock.QueryMatcherEqual))
			require.NoError(t, err)
			db := bun.NewDB(sqlDB, pgdialect.New())
			t.Cleanup(func() { _ = db.Close() })

			type row struct {
				bun.BaseModel `bun:"table:query_test,alias:qt"`
				ID            int `bun:"id"`
			}
			var rows []row
			if tc.total == 0 {
				mock.ExpectQuery(`SELECT count(*) FROM "query_test" AS "qt"`).
					WillReturnRows(sqlmock.NewRows([]string{"count"}).AddRow(tc.count))
			}
			mock.ExpectQuery(`SELECT "qt"."id" FROM "query_test" AS "qt" ORDER BY "id" ASC LIMIT 2 OFFSET 1`).
				WillReturnRows(sqlmock.NewRows([]string{"id"}).AddRow(2).AddRow(3))

			query, err := New(t.Context(), &Config{
				IDB:            db,
				Model:          &rows,
				Pagination:     &Pagination{Offset: 1, Limit: 2, Total: tc.total},
				DefaultOrderBy: []OrderBy{{Column: "id", Direction: OrderAscending}},
			})
			require.NoError(t, err)
			assert.Equal(t, tc.count, query.TotalCount())
			err = query.Scan(t.Context())
			require.NoError(t, err)
			require.Len(t, rows, 2)
			assert.Equal(t, 2, rows[0].ID)
			assert.Equal(t, 3, rows[1].ID)
			require.NoError(t, mock.ExpectationsWereMet())
		})
	}
}
