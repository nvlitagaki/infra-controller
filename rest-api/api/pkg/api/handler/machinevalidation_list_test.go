// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

package handler

import (
	"encoding/json"
	"net/http"
	"net/url"
	"testing"
	"time"

	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
	"google.golang.org/protobuf/encoding/protojson"
	"google.golang.org/protobuf/types/known/timestamppb"

	"github.com/NVIDIA/infra-controller/rest-api/api/pkg/api/model"
	corev1 "github.com/NVIDIA/infra-controller/rest-api/proto/core/gen/v1"
)

func TestListMachineValidationRunsHandlerProxiesFiltersAndPagination(t *testing.T) {
	startedAt := time.Date(2026, time.September, 17, 12, 0, 0, 0, time.UTC)
	fixture := newCredentialRotationHandlerFixture(t, &corev1.ListMachineValidationRunsResponse{
		Runs: []*corev1.MachineValidationRun{
			{
				ValidationId: &corev1.MachineValidationId{Value: "validation-1"},
				MachineId:    &corev1.MachineId{Id: "machine-1"},
				StartTime:    timestamppb.New(startedAt),
			},
		},
		NextPageToken: "next-token",
		TotalSize:     42,
	})
	handler := NewListMachineValidationRunsHandler(fixture.dbSession, fixture.scp)
	query := url.Values{
		"siteId":        []string{fixture.siteID},
		"machineId":     []string{"machine-1"},
		"startedAfter":  []string{"2026-09-01T00:00:00Z"},
		"startedBefore": []string{"2026-10-01T00:00:00Z"},
		"state":         []string{"Failed"},
		"pageSize":      []string{"2"},
		"pageToken":     []string{"current-token"},
	}

	rec := fixture.get(t, handler.Handle, query)

	assert.Equal(t, http.StatusOK, rec.Code)
	assert.Equal(t, corev1.Forge_ListMachineValidationRuns_FullMethodName, fixture.proxiedReq.FullMethod)
	var coreRequest corev1.ListMachineValidationRunsRequest
	err := protojson.Unmarshal(fixture.proxiedReq.RequestJSON, &coreRequest)
	require.NoError(t, err)
	assert.Equal(t, "machine-1", coreRequest.GetMachineId().GetId())
	assert.Equal(t, uint32(2), coreRequest.GetPageSize())
	assert.Equal(t, "current-token", coreRequest.GetPageToken())
	assert.Equal(t, corev1.MachineValidationRunState_MACHINE_VALIDATION_RUN_STATE_FAILED, coreRequest.GetState())
	assert.Equal(t, "2026-09-01T00:00:00Z", coreRequest.GetStartedAfter().AsTime().Format(time.RFC3339))
	assert.Equal(t, "2026-10-01T00:00:00Z", coreRequest.GetStartedBefore().AsTime().Format(time.RFC3339))

	var runs []model.APIMachineValidationRun
	err = json.Unmarshal(rec.Body.Bytes(), &runs)
	require.NoError(t, err)
	require.Len(t, runs, 1)
	assert.Equal(t, "validation-1", runs[0].ValidationID)
	assert.JSONEq(t, `{"pageSize":2,"total":42,"nextPageToken":"next-token"}`, rec.Header().Get("X-Pagination"))
}

func TestListMachineValidationRunsHandlerRejectsInvalidState(t *testing.T) {
	fixture := newCredentialRotationHandlerFixture(t, nil)
	handler := NewListMachineValidationRunsHandler(fixture.dbSession, fixture.scp)
	query := url.Values{
		"siteId": []string{fixture.siteID},
		"state":  []string{"Completed"},
	}

	rec := fixture.get(t, handler.Handle, query)

	assert.Equal(t, http.StatusBadRequest, rec.Code)
	assert.Empty(t, fixture.proxiedReq.FullMethod)
}

func TestListMachineValidationRunsHandlerRejectsUnboundedPageSize(t *testing.T) {
	fixture := newCredentialRotationHandlerFixture(t, nil)
	handler := NewListMachineValidationRunsHandler(fixture.dbSession, fixture.scp)
	query := url.Values{
		"siteId":   []string{fixture.siteID},
		"pageSize": []string{"101"},
	}

	rec := fixture.get(t, handler.Handle, query)

	assert.Equal(t, http.StatusBadRequest, rec.Code)
	assert.Empty(t, fixture.proxiedReq.FullMethod)
}

func TestListMachineValidationRunsHandlerUsesBoundedDefault(t *testing.T) {
	fixture := newCredentialRotationHandlerFixture(t, &corev1.ListMachineValidationRunsResponse{})
	handler := NewListMachineValidationRunsHandler(fixture.dbSession, fixture.scp)
	query := url.Values{
		"siteId": []string{fixture.siteID},
	}

	rec := fixture.get(t, handler.Handle, query)

	assert.Equal(t, http.StatusOK, rec.Code)
	assert.JSONEq(t, `{"pageSize":20,"total":0}`, rec.Header().Get("X-Pagination"))
	var coreRequest corev1.ListMachineValidationRunsRequest
	err := protojson.Unmarshal(fixture.proxiedReq.RequestJSON, &coreRequest)
	require.NoError(t, err)
	assert.Equal(t, uint32(0), coreRequest.GetPageSize())
}
